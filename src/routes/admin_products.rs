use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::auth::AuthCreator;
use crate::services::product::{AdminProductListItem, ProductService};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub category: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub products: Vec<AdminProductListItem>,
    pub total: i64,
    pub page: i64,
    pub limit: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateProductRequest {
    pub title: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub images: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct ProductResponse {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub category: String,
    pub images: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ProductDetailResponse {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub category: String,
    pub images: serde_json::Value,
    pub merchant_links: Vec<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProductRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub images: Option<Vec<String>>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/products", get(list).post(create))
        .route("/products/{id}", get(get_one).put(update).delete(delete))
}

pub async fn list(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Query(params): Query<ListParams>,
) -> Result<(StatusCode, Json<ListResponse>), AppError> {
    let page = params.page.unwrap_or(1).max(1);
    let limit = params.limit.unwrap_or(20).clamp(1, 100);

    let (products, total) = ProductService::list_admin_products(
        &state.db,
        creator_id,
        params.category,
        params.search,
        page,
        limit,
    )
    .await?;

    Ok((
        StatusCode::OK,
        Json(ListResponse {
            products,
            total,
            page,
            limit,
        }),
    ))
}

pub async fn create(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Json(payload): Json<CreateProductRequest>,
) -> Result<(StatusCode, Json<ProductResponse>), AppError> {
    if payload.title.trim().is_empty() || payload.title.len() > 500 {
        return Err(AppError::Validation(
            "title is required and must be at most 500 chars".to_string(),
        ));
    }

    let description = payload.description.unwrap_or_default();
    let category = payload
        .category
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| "uncategorized".to_string());
    let images = serde_json::to_value(payload.images.unwrap_or_default())
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let product = ProductService::create_product(
        &state.db,
        creator_id,
        payload.title,
        description,
        category,
        images,
    )
    .await?;

    // Invalidate store cache (write-through)
    state.invalidate_store_cache(None).await;

    Ok((
        StatusCode::CREATED,
        Json(ProductResponse {
            id: product.id,
            title: product.title,
            description: product.description,
            category: product.category,
            images: product.images,
            created_at: product.created_at,
        }),
    ))
}

pub async fn get_one(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(product_id): Path<Uuid>,
) -> Result<Json<ProductDetailResponse>, AppError> {
    let (product, merchant_links) =
        ProductService::get_admin_product(&state.db, creator_id, product_id).await?;

    Ok(Json(ProductDetailResponse {
        id: product.id,
        title: product.title,
        description: product.description,
        category: product.category,
        images: product.images,
        merchant_links,
        created_at: product.created_at,
        updated_at: product.updated_at,
    }))
}

pub async fn update(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(product_id): Path<Uuid>,
    Json(payload): Json<UpdateProductRequest>,
) -> Result<Json<ProductResponse>, AppError> {
    if let Some(ref title) = payload.title {
        if title.trim().is_empty() || title.len() > 500 {
            return Err(AppError::Validation("title must be 1-500 chars".to_string()));
        }
    }

    let images_val = match payload.images {
        Some(images) => Some(
            serde_json::to_value(images).map_err(|e| AppError::Internal(e.to_string()))?,
        ),
        None => None,
    };

    let updated = ProductService::update_product(
        &state.db,
        creator_id,
        product_id,
        payload.title,
        payload.description,
        payload.category,
        images_val,
    )
    .await?;

    // Invalidate store cache (write-through)
    state.invalidate_store_cache(Some(product_id)).await;

    Ok(Json(ProductResponse {
        id: updated.id,
        title: updated.title,
        description: updated.description,
        category: updated.category,
        images: updated.images,
        created_at: updated.created_at,
    }))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(product_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    ProductService::delete_product(&state.db, creator_id, product_id).await?;

    // Invalidate store cache (write-through)
    state.invalidate_store_cache(Some(product_id)).await;

    Ok(Json(serde_json::json!({ "message": "Product deleted" })))
}
