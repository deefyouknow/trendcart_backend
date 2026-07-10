use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::auth::AuthCreator;
use crate::models::merchant_link::MerchantLink;
use crate::models::product::Product;
use crate::models::product_variant::ProductVariant;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateMerchantLinkRequest {
    pub platform: String,
    pub store_name: String,
    pub affiliate_url: String,
    pub variants: Vec<CreateVariantRequest>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVariantRequest {
    pub variant_name: String,
    pub price: f64,
    pub currency: Option<String>,
    pub stock_status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MerchantLinkResponse {
    pub id: Uuid,
    pub product_id: Uuid,
    pub platform: String,
    pub store_name: String,
    pub affiliate_url: String,
    pub is_price_estimated: bool,
    pub price_checked_at: Option<DateTime<Utc>>,
    pub variants: Vec<VariantResponse>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct VariantResponse {
    pub id: Uuid,
    pub variant_name: String,
    pub price: f64,
    pub currency: String,
    pub stock_status: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMerchantLinkRequest {
    pub platform: Option<String>,
    pub store_name: Option<String>,
    pub affiliate_url: Option<String>,
    pub is_price_estimated: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct VerifyPriceRequest {
    pub is_price_estimated: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateVariantRequest {
    pub variant_name: Option<String>,
    pub price: Option<f64>,
    pub currency: Option<String>,
    pub stock_status: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/products/{product_id}/merchant-links",
            post(create),
        )
        .route(
            "/merchant-links/{id}",
            get(get_one).put(update).delete(delete_link),
        )
        .route(
            "/merchant-links/{id}/verify-price",
            put(verify_price),
        )
        .route(
            "/merchant-links/{merchant_link_id}/variants",
            post(create_variant),
        )
        .route(
            "/variants/{variant_id}",
            put(update_variant).delete(delete_variant),
        )
}

/// Helper: verify creator owns the product
async fn verify_product_ownership(
    state: &AppState,
    product_id: Uuid,
    creator_id: Uuid,
) -> Result<Product, AppError> {
    let product: Option<Product> =
        sqlx::query_as("SELECT * FROM products WHERE id = $1 AND deleted_at IS NULL")
            .bind(product_id)
            .fetch_optional(&state.db)
            .await?;

    let product = product.ok_or_else(|| AppError::NotFound("Product not found".to_string()))?;

    if product.creator_id != creator_id {
        return Err(AppError::Forbidden);
    }

    Ok(product)
}

/// Helper: verify creator owns the merchant link (via product ownership)
async fn verify_merchant_link_ownership(
    state: &AppState,
    merchant_link_id: Uuid,
    creator_id: Uuid,
) -> Result<MerchantLink, AppError> {
    let link: Option<MerchantLink> =
        sqlx::query_as("SELECT * FROM merchant_links WHERE id = $1")
            .bind(merchant_link_id)
            .fetch_optional(&state.db)
            .await?;

    let link = link.ok_or_else(|| AppError::NotFound("Merchant link not found".to_string()))?;

    verify_product_ownership(state, link.product_id, creator_id).await?;

    Ok(link)
}

/// POST /api/admin/products/{product_id}/merchant-links
pub async fn create(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(product_id): Path<Uuid>,
    Json(payload): Json<CreateMerchantLinkRequest>,
) -> Result<(StatusCode, Json<MerchantLinkResponse>), AppError> {
    // Validate inputs
    let platform_lower = payload.platform.trim().to_lowercase();
    if platform_lower.is_empty() {
        return Err(AppError::Validation("platform is required".to_string()));
    }
    let valid_platforms = ["shopee", "lazada", "tiktok", "other"];
    if !valid_platforms.contains(&platform_lower.as_str()) {
        return Err(AppError::Validation(format!(
            "platform must be one of: {}",
            valid_platforms.join(", ")
        )));
    }
    if payload.store_name.trim().is_empty() {
        return Err(AppError::Validation("store_name is required".to_string()));
    }
    if payload.affiliate_url.trim().is_empty() {
        return Err(AppError::Validation("affiliate_url is required".to_string()));
    }
    if payload.variants.is_empty() {
        return Err(AppError::Validation(
            "at least one variant is required".to_string(),
        ));
    }

    // Verify product ownership
    verify_product_ownership(&state, product_id, creator_id).await?;

    // Validate stock_status values before starting transaction
    for v in &payload.variants {
        let stock_status = v.stock_status.as_deref().unwrap_or("unknown");
        if !["in_stock", "out_of_stock", "unknown"].contains(&stock_status) {
            return Err(AppError::Validation(
                "stock_status must be one of: in_stock, out_of_stock, unknown".to_string(),
            ));
        }
    }

    // Use transaction to prevent race condition between verify and insert
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| AppError::Internal(format!("Transaction error: {}", e)))?;

    // Create merchant link
    let link: MerchantLink = sqlx::query_as(
        r#"
        INSERT INTO merchant_links (product_id, platform, store_name, affiliate_url)
        VALUES ($1, $2, $3, $4)
        RETURNING *
        "#,
    )
    .bind(product_id)
    .bind(&platform_lower)
    .bind(&payload.store_name)
    .bind(&payload.affiliate_url)
    .fetch_one(&mut *tx)
    .await?;

    // Create variants
    let mut variants = Vec::new();
    for v in &payload.variants {
        let currency = v.currency.clone().unwrap_or_else(|| "THB".to_string());
        let stock_status = v.stock_status.clone().unwrap_or_else(|| "unknown".to_string());

        let variant: ProductVariant = sqlx::query_as(
            r#"
            INSERT INTO product_variants (merchant_link_id, variant_name, price, currency, stock_status)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(link.id)
        .bind(&v.variant_name)
        .bind(v.price)
        .bind(&currency)
        .bind(&stock_status)
        .fetch_one(&mut *tx)
        .await?;

        variants.push(VariantResponse {
            id: variant.id,
            variant_name: variant.variant_name,
            price: variant.price,
            currency: variant.currency,
            stock_status: variant.stock_status,
        });
    }

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("Transaction commit error: {}", e)))?;

    // Invalidate store cache (write-through)
    state.invalidate_store_cache(Some(product_id)).await;

    Ok((
        StatusCode::CREATED,
        Json(MerchantLinkResponse {
            id: link.id,
            product_id: link.product_id,
            platform: link.platform,
            store_name: link.store_name,
            affiliate_url: link.affiliate_url,
            is_price_estimated: link.is_price_estimated,
            price_checked_at: link.price_checked_at,
            variants,
            created_at: None,
            updated_at: link.updated_at,
        }),
    ))
}

/// GET /api/admin/merchant-links/{id}
pub async fn get_one(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(merchant_link_id): Path<Uuid>,
) -> Result<Json<MerchantLinkResponse>, AppError> {
    let link = verify_merchant_link_ownership(&state, merchant_link_id, creator_id).await?;

    let variants: Vec<ProductVariant> = sqlx::query_as(
        "SELECT * FROM product_variants WHERE merchant_link_id = $1 ORDER BY variant_name",
    )
    .bind(merchant_link_id)
    .fetch_all(&state.db)
    .await?;

    let variant_responses: Vec<VariantResponse> = variants
        .into_iter()
        .map(|v| VariantResponse {
            id: v.id,
            variant_name: v.variant_name,
            price: v.price,
            currency: v.currency,
            stock_status: v.stock_status,
        })
        .collect();

    Ok(Json(MerchantLinkResponse {
        id: link.id,
        product_id: link.product_id,
        platform: link.platform,
        store_name: link.store_name,
        affiliate_url: link.affiliate_url,
        is_price_estimated: link.is_price_estimated,
        price_checked_at: link.price_checked_at,
        variants: variant_responses,
        created_at: None,
        updated_at: link.updated_at,
    }))
}

/// PUT /api/admin/merchant-links/{id}
pub async fn update(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(merchant_link_id): Path<Uuid>,
    Json(payload): Json<UpdateMerchantLinkRequest>,
) -> Result<Json<MerchantLinkResponse>, AppError> {
    let existing = verify_merchant_link_ownership(&state, merchant_link_id, creator_id).await?;

    let mut updated_platform = None;
    if let Some(ref platform) = payload.platform {
        let platform_lower = platform.trim().to_lowercase();
        let valid_platforms = ["shopee", "lazada", "tiktok", "other"];
        if !valid_platforms.contains(&platform_lower.as_str()) {
            return Err(AppError::Validation(format!(
                "platform must be one of: {}",
                valid_platforms.join(", ")
            )));
        }
        updated_platform = Some(platform_lower);
    }

    let platform = updated_platform.unwrap_or(existing.platform);
    let store_name = payload.store_name.unwrap_or(existing.store_name);
    let affiliate_url = payload.affiliate_url.unwrap_or(existing.affiliate_url);
    let is_price_estimated = payload.is_price_estimated.unwrap_or(existing.is_price_estimated);

    let updated: MerchantLink = sqlx::query_as(
        r#"
        UPDATE merchant_links
        SET platform = $1, store_name = $2, affiliate_url = $3, is_price_estimated = $4
        WHERE id = $5
        RETURNING *
        "#,
    )
    .bind(&platform)
    .bind(&store_name)
    .bind(&affiliate_url)
    .bind(is_price_estimated)
    .bind(merchant_link_id)
    .fetch_one(&state.db)
    .await?;

    // Fetch variants
    let variants: Vec<ProductVariant> = sqlx::query_as(
        "SELECT * FROM product_variants WHERE merchant_link_id = $1 ORDER BY variant_name",
    )
    .bind(merchant_link_id)
    .fetch_all(&state.db)
    .await?;

    let variant_responses: Vec<VariantResponse> = variants
        .into_iter()
        .map(|v| VariantResponse {
            id: v.id,
            variant_name: v.variant_name,
            price: v.price,
            currency: v.currency,
            stock_status: v.stock_status,
        })
        .collect();

    // Invalidate store cache (write-through)
    state.invalidate_store_cache(Some(updated.product_id)).await;

    Ok(Json(MerchantLinkResponse {
        id: updated.id,
        product_id: updated.product_id,
        platform: updated.platform,
        store_name: updated.store_name,
        affiliate_url: updated.affiliate_url,
        is_price_estimated: updated.is_price_estimated,
        price_checked_at: updated.price_checked_at,
        variants: variant_responses,
        created_at: None,
        updated_at: updated.updated_at,
    }))
}

/// DELETE /api/admin/merchant-links/{id}
pub async fn delete_link(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(merchant_link_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let link = verify_merchant_link_ownership(&state, merchant_link_id, creator_id).await?;
    let product_id = link.product_id;

    sqlx::query("DELETE FROM merchant_links WHERE id = $1")
        .bind(merchant_link_id)
        .execute(&state.db)
        .await?;

    // Invalidate store cache (write-through)
    state.invalidate_store_cache(Some(product_id)).await;

    Ok(Json(serde_json::json!({ "message": "Merchant link deleted" })))
}

/// PUT /api/admin/merchant-links/{id}/verify-price
pub async fn verify_price(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(merchant_link_id): Path<Uuid>,
    Json(payload): Json<VerifyPriceRequest>,
) -> Result<Json<MerchantLinkResponse>, AppError> {
    let _existing = verify_merchant_link_ownership(&state, merchant_link_id, creator_id).await?;

    let is_price_estimated = payload.is_price_estimated.unwrap_or(false);

    let updated: MerchantLink = sqlx::query_as(
        r#"
        UPDATE merchant_links
        SET is_price_estimated = $1, price_checked_at = NOW()
        WHERE id = $2
        RETURNING *
        "#,
    )
    .bind(is_price_estimated)
    .bind(merchant_link_id)
    .fetch_one(&state.db)
    .await?;

    // Fetch variants
    let variants: Vec<ProductVariant> = sqlx::query_as(
        "SELECT * FROM product_variants WHERE merchant_link_id = $1 ORDER BY variant_name",
    )
    .bind(merchant_link_id)
    .fetch_all(&state.db)
    .await?;

    let variant_responses: Vec<VariantResponse> = variants
        .into_iter()
        .map(|v| VariantResponse {
            id: v.id,
            variant_name: v.variant_name,
            price: v.price,
            currency: v.currency,
            stock_status: v.stock_status,
        })
        .collect();

    // Invalidate store cache (write-through)
    state.invalidate_store_cache(Some(updated.product_id)).await;

    Ok(Json(MerchantLinkResponse {
        id: updated.id,
        product_id: updated.product_id,
        platform: updated.platform,
        store_name: updated.store_name,
        affiliate_url: updated.affiliate_url,
        is_price_estimated: updated.is_price_estimated,
        price_checked_at: updated.price_checked_at,
        variants: variant_responses,
        created_at: None,
        updated_at: updated.updated_at,
    }))
}

/// Helper: verify creator owns the variant (via merchant link → product)
async fn verify_variant_ownership(
    state: &AppState,
    variant_id: Uuid,
    creator_id: Uuid,
) -> Result<ProductVariant, AppError> {
    let variant: Option<ProductVariant> =
        sqlx::query_as("SELECT * FROM product_variants WHERE id = $1")
            .bind(variant_id)
            .fetch_optional(&state.db)
            .await?;

    let variant = variant.ok_or_else(|| AppError::NotFound("Variant not found".to_string()))?;

    verify_merchant_link_ownership(state, variant.merchant_link_id, creator_id).await?;

    Ok(variant)
}

/// POST /api/admin/merchant-links/{merchant_link_id}/variants
pub async fn create_variant(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(merchant_link_id): Path<Uuid>,
    Json(payload): Json<CreateVariantRequest>,
) -> Result<(StatusCode, Json<VariantResponse>), AppError> {
    let link = verify_merchant_link_ownership(&state, merchant_link_id, creator_id).await?;

    if payload.variant_name.trim().is_empty() {
        return Err(AppError::Validation("variant_name is required".to_string()));
    }
    if payload.price < 0.0 {
        return Err(AppError::Validation("price must be non-negative".to_string()));
    }

    let currency = payload.currency.unwrap_or_else(|| "THB".to_string());
    let stock_status = payload.stock_status.unwrap_or_else(|| "unknown".to_string());

    if !["in_stock", "out_of_stock", "unknown"].contains(&stock_status.as_str()) {
        return Err(AppError::Validation(
            "stock_status must be one of: in_stock, out_of_stock, unknown".to_string(),
        ));
    }

    let variant: ProductVariant = sqlx::query_as(
        r#"
        INSERT INTO product_variants (merchant_link_id, variant_name, price, currency, stock_status)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING *
        "#,
    )
    .bind(merchant_link_id)
    .bind(&payload.variant_name)
    .bind(payload.price)
    .bind(&currency)
    .bind(&stock_status)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.code().map_or(false, |c| c == "23503") {
                return AppError::NotFound(
                    "Merchant link was deleted before variant could be created".to_string(),
                );
            }
        }
        AppError::from(e)
    })?;

    // Invalidate store cache (write-through)
    state.invalidate_store_cache(Some(link.product_id)).await;

    Ok((
        StatusCode::CREATED,
        Json(VariantResponse {
            id: variant.id,
            variant_name: variant.variant_name,
            price: variant.price,
            currency: variant.currency,
            stock_status: variant.stock_status,
        }),
    ))
}

/// PUT /api/admin/variants/{variant_id}
pub async fn update_variant(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(variant_id): Path<Uuid>,
    Json(payload): Json<UpdateVariantRequest>,
) -> Result<Json<VariantResponse>, AppError> {
    let existing = verify_variant_ownership(&state, variant_id, creator_id).await?;

    if let Some(ref stock) = payload.stock_status {
        if !["in_stock", "out_of_stock", "unknown"].contains(&stock.as_str()) {
            return Err(AppError::Validation(
                "stock_status must be one of: in_stock, out_of_stock, unknown".to_string(),
            ));
        }
    }
    if let Some(p) = payload.price {
        if p < 0.0 {
            return Err(AppError::Validation("price must be non-negative".to_string()));
        }
    }

    let variant_name = payload.variant_name.unwrap_or(existing.variant_name);
    let price = payload.price.unwrap_or(existing.price);
    let currency = payload.currency.unwrap_or(existing.currency);
    let stock_status = payload.stock_status.unwrap_or(existing.stock_status);

    let updated: ProductVariant = sqlx::query_as(
        r#"
        UPDATE product_variants
        SET variant_name = $1, price = $2, currency = $3, stock_status = $4
        WHERE id = $5
        RETURNING *
        "#,
    )
    .bind(&variant_name)
    .bind(price)
    .bind(&currency)
    .bind(&stock_status)
    .bind(variant_id)
    .fetch_one(&state.db)
    .await?;

    // Invalidate store cache (variant price change affects product listing)
    let product_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT ml.product_id FROM merchant_links ml WHERE ml.id = $1",
    )
    .bind(existing.merchant_link_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    state.invalidate_store_cache(product_id).await;

    Ok(Json(VariantResponse {
        id: updated.id,
        variant_name: updated.variant_name,
        price: updated.price,
        currency: updated.currency,
        stock_status: updated.stock_status,
    }))
}

/// DELETE /api/admin/variants/{variant_id}
pub async fn delete_variant(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(variant_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let existing = verify_variant_ownership(&state, variant_id, creator_id).await?;

    sqlx::query("DELETE FROM product_variants WHERE id = $1")
        .bind(variant_id)
        .execute(&state.db)
        .await?;

    // Invalidate store cache
    let product_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT ml.product_id FROM merchant_links ml WHERE ml.id = $1",
    )
    .bind(existing.merchant_link_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    state.invalidate_store_cache(product_id).await;

    Ok(Json(serde_json::json!({ "message": "Variant deleted" })))
}
