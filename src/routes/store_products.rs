use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json;
use uuid::Uuid;

use crate::db::query_with_retry;
use crate::error::AppError;
use crate::models::merchant_link::MerchantLink;
use crate::models::product::Product;
use crate::models::product_variant::ProductVariant;
use crate::state::AppState;

#[derive(Debug, sqlx::FromRow, Serialize, Deserialize)]
pub struct StoreProductListItem {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub category: String,
    pub images: serde_json::Value,
    pub min_price: Option<f64>,
    pub min_currency: Option<String>,
    pub platforms: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct StoreListParams {
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub category: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StoreListResponse {
    pub products: Vec<StoreProductListItem>,
    pub total: i64,
    pub page: i64,
    pub limit: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StoreProductDetailResponse {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub category: String,
    pub images: serde_json::Value,
    pub merchant_links: Vec<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CategoryResponse {
    pub name: String,
    pub product_count: i64,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/categories", get(list_categories))
        .route("/{id}", get(get_one))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<StoreListParams>,
) -> Result<Json<StoreListResponse>, AppError> {
    let page = params.page.unwrap_or(1).max(1);
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * limit;

    // Build cache key from query params
    let cache_key = format!(
        "store:list:{}:{}:{:?}:{:?}",
        page, limit, params.category, params.search
    );

    // Check cache first (Redis)
    if let Some(cached_json) = state.redis.get(&cache_key).await {
        if let Ok(response) = serde_json::from_str::<StoreListResponse>(&cached_json) {
            return Ok(Json(response));
        }
    }

    let products: Vec<StoreProductListItem> = query_with_retry(|| {
        let category = params.category.clone();
        let search = params.search.clone();
        let db = state.db.clone();
        async move {
            sqlx::query_as(
                r#"
                SELECT
                    p.id,
                    p.title,
                    p.description,
                    p.category,
                    p.images,
                    MIN(pv.price)::float8 AS min_price,
                    (array_agg(DISTINCT pv.currency))[1] AS min_currency,
                    COALESCE(
                        array_agg(DISTINCT ml.platform::text) FILTER (WHERE ml.platform IS NOT NULL),
                        ARRAY[]::text[]
                    ) AS platforms,
                    p.created_at
                FROM products p
                LEFT JOIN merchant_links ml ON ml.product_id = p.id
                LEFT JOIN product_variants pv ON pv.merchant_link_id = ml.id
                WHERE p.deleted_at IS NULL
                  AND ($1::text IS NULL OR p.category = $1)
                  AND ($2::text IS NULL OR p.search_vector @@ plainto_tsquery('simple', $2))
                GROUP BY p.id
                ORDER BY p.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(&category)
            .bind(&search)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
    })
    .await?;

    let total: i64 = query_with_retry(|| {
        let category = params.category.clone();
        let search = params.search.clone();
        let db = state.db.clone();
        async move {
            sqlx::query_scalar(
                r#"
                SELECT COUNT(*)
                FROM products
                WHERE deleted_at IS NULL
                  AND ($1::text IS NULL OR category = $1)
                  AND ($2::text IS NULL OR search_vector @@ plainto_tsquery('simple', $2))
                "#,
            )
            .bind(&category)
            .bind(&search)
            .fetch_one(&db)
            .await
        }
    })
    .await?;

    let response = StoreListResponse {
        products,
        total,
        page,
        limit,
    };

    // Cache the result in Redis (300s TTL)
    if let Ok(json) = serde_json::to_string(&response) {
        state.redis.set(&cache_key, &json, 300).await;
    }

    Ok(Json(response))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(product_id): Path<Uuid>,
) -> Result<Json<StoreProductDetailResponse>, AppError> {
    let cache_key = format!("store:product:{}", product_id);

    // Check Redis cache first
    if let Some(cached_json) = state.redis.get(&cache_key).await {
        if let Ok(response) = serde_json::from_str::<StoreProductDetailResponse>(&cached_json) {
            return Ok(Json(response));
        }
    }

    let product: Option<Product> = query_with_retry(|| {
        let pid = product_id;
        let db = state.db.clone();
        async move {
            sqlx::query_as("SELECT * FROM products WHERE id = $1 AND deleted_at IS NULL")
                .bind(pid)
                .fetch_optional(&db)
                .await
        }
    })
    .await?;

    let product = product.ok_or_else(|| AppError::NotFound("Product not found".to_string()))?;

    // Fetch merchant links for this product
    let merchant_links: Vec<MerchantLink> = query_with_retry(|| {
        let pid = product_id;
        let db = state.db.clone();
        async move {
            sqlx::query_as("SELECT * FROM merchant_links WHERE product_id = $1 ORDER BY updated_at")
                .bind(pid)
                .fetch_all(&db)
                .await
        }
    })
    .await?;

    // Fetch variants for each merchant link
    let mut merchant_links_response = Vec::new();
    for link in merchant_links {
        let link_id = link.id;
        let variants: Vec<ProductVariant> = query_with_retry(|| {
            let lid = link_id;
            let db = state.db.clone();
            async move {
                sqlx::query_as(
                    "SELECT * FROM product_variants WHERE merchant_link_id = $1 ORDER BY variant_name",
                )
                .bind(lid)
                .fetch_all(&db)
                .await
            }
        })
        .await?;

        let mut min_price = f64::INFINITY;
        let mut currency = "THB".to_string();
        for v in &variants {
            if v.price < min_price {
                min_price = v.price;
                currency = v.currency.clone();
            }
        }
        if min_price == f64::INFINITY {
            min_price = 0.0;
        }

        merchant_links_response.push(serde_json::json!({
            "id": link.id,
            "platform": link.platform,
            "store_name": link.store_name,
            "platform_price": min_price,
            "currency": currency,
            "affiliate_url": link.affiliate_url,
            "is_price_estimated": link.is_price_estimated,
            "price_checked_at": link.price_checked_at,
            "variants": variants,
        }));
    }

    let response = StoreProductDetailResponse {
        id: product.id,
        title: product.title,
        description: product.description,
        category: product.category,
        images: product.images,
        merchant_links: merchant_links_response,
        created_at: product.created_at,
    };

    // Cache in Redis (300s TTL)
    if let Ok(json) = serde_json::to_string(&response) {
        state.redis.set(&cache_key, &json, 300).await;
    }

    Ok(Json(response))
}

pub async fn list_categories(
    State(state): State<AppState>,
) -> Result<Json<Vec<CategoryResponse>>, AppError> {
    let categories: Vec<CategoryResponse> = query_with_retry(|| {
        let db = state.db.clone();
        async move {
            sqlx::query_as(
                r#"
                SELECT category AS name, COUNT(*) AS product_count
                FROM products
                WHERE deleted_at IS NULL
                GROUP BY category
                ORDER BY product_count DESC
                "#,
            )
            .fetch_all(&db)
            .await
        }
    })
    .await?;

    Ok(Json(categories))
}
