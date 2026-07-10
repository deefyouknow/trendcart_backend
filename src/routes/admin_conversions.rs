use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::auth::AuthCreator;
use crate::models::conversion::Conversion;
use crate::models::merchant_link::MerchantLink;
use crate::routes::common::PaginatedResponse;
use crate::state::AppState;

// --- Request/Response Types ---

#[derive(Debug, Deserialize)]
pub struct ConversionListParams {
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub status: Option<String>,
    pub platform: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConversionResponse {
    pub id: Uuid,
    pub click_id: Uuid,
    pub merchant_link_id: Uuid,
    pub platform: String,
    pub order_id: Option<String>,
    pub order_amount: Option<f64>,
    pub currency: String,
    pub commission: Option<f64>,
    pub status: String,
    pub conversion_type: String,
    pub product_name: Option<String>,
    pub raw_data: serde_json::Value,
    pub postback_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl From<Conversion> for ConversionResponse {
    fn from(c: Conversion) -> Self {
        Self {
            id: c.id,
            click_id: c.click_id,
            merchant_link_id: c.merchant_link_id,
            platform: c.platform,
            order_id: c.order_id,
            order_amount: c.order_amount,
            currency: c.currency,
            commission: c.commission,
            status: c.status,
            conversion_type: c.conversion_type,
            product_name: c.product_name,
            raw_data: c.raw_data,
            postback_at: c.postback_at,
            created_at: c.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateConversionRequest {
    pub status: String, // 'approved', 'rejected', 'cancelled'
}

// --- Postback Request (for platforms to report conversions) ---

#[derive(Debug, Deserialize)]
pub struct PostbackRequest {
    pub click_id: String,
    pub order_id: Option<String>,
    pub order_amount: Option<f64>,
    pub currency: Option<String>,
    pub commission: Option<f64>,
    pub platform: String,
    pub conversion_type: Option<String>,
    pub product_name: Option<String>,
    pub raw_data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct PostbackResponse {
    pub conversion_id: Uuid,
    pub status: String,
}

// --- Router ---

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/conversions", get(list_conversions))
        .route("/conversions/{id}", get(get_conversion).put(update_conversion))
        .route("/conversions/{id}/approve", post(approve_conversion))
        .route("/conversions/{id}/reject", post(reject_conversion))
}

pub fn postback_router() -> Router<AppState> {
    Router::new().route("/postback", post(handle_postback))
}

// --- Admin Handlers ---

pub async fn list_conversions(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Query(params): Query<ConversionListParams>,
) -> Result<(StatusCode, Json<PaginatedResponse<ConversionResponse>>), AppError> {
    let pagination = crate::routes::common::PaginationParams {
        page: params.page,
        limit: params.limit,
    };
    let (page, limit, offset) = crate::routes::common::parse_pagination(&pagination);

    let conversions: Vec<Conversion> = sqlx::query_as(
        r#"
        SELECT * FROM conversions
        WHERE creator_id = $1
          AND ($2::text IS NULL OR status = $2)
          AND ($3::text IS NULL OR platform = $3)
        ORDER BY created_at DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(creator_id)
    .bind(&params.status)
    .bind(&params.platform)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM conversions
        WHERE creator_id = $1
          AND ($2::text IS NULL OR status = $2)
          AND ($3::text IS NULL OR platform = $3)
        "#,
    )
    .bind(creator_id)
    .bind(&params.status)
    .bind(&params.platform)
    .fetch_one(&state.db)
    .await?;

    let items: Vec<ConversionResponse> = conversions
        .into_iter()
        .map(ConversionResponse::from)
        .collect();

    Ok((
        StatusCode::OK,
        Json(PaginatedResponse::new(items, total, page, limit)),
    ))
}

pub async fn get_conversion(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(conversion_id): Path<Uuid>,
) -> Result<Json<ConversionResponse>, AppError> {
    let conversion: Option<Conversion> =
        sqlx::query_as("SELECT * FROM conversions WHERE id = $1")
            .bind(conversion_id)
            .fetch_optional(&state.db)
            .await?;

    let conversion = conversion.ok_or_else(|| AppError::NotFound("Conversion not found".to_string()))?;

    if conversion.creator_id != creator_id {
        return Err(AppError::Forbidden);
    }

    Ok(Json(ConversionResponse::from(conversion)))
}

pub async fn update_conversion(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(conversion_id): Path<Uuid>,
    Json(payload): Json<UpdateConversionRequest>,
) -> Result<Json<ConversionResponse>, AppError> {
    // Validate status
    let valid_statuses = ["pending", "approved", "rejected", "cancelled"];
    if !valid_statuses.contains(&payload.status.as_str()) {
        return Err(AppError::Validation(
            "Invalid status. Must be: pending, approved, rejected, or cancelled".to_string(),
        ));
    }

    let existing: Option<Conversion> =
        sqlx::query_as("SELECT * FROM conversions WHERE id = $1")
            .bind(conversion_id)
            .fetch_optional(&state.db)
            .await?;

    let existing = existing.ok_or_else(|| AppError::NotFound("Conversion not found".to_string()))?;

    if existing.creator_id != creator_id {
        return Err(AppError::Forbidden);
    }

    let updated: Conversion = sqlx::query_as(
        r#"
        UPDATE conversions
        SET status = $1
        WHERE id = $2
        RETURNING *
        "#,
    )
    .bind(&payload.status)
    .bind(conversion_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(ConversionResponse::from(updated)))
}

pub async fn approve_conversion(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(conversion_id): Path<Uuid>,
) -> Result<Json<ConversionResponse>, AppError> {
    approve_or_reject(state, creator_id, conversion_id, "approved").await
}

pub async fn reject_conversion(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(conversion_id): Path<Uuid>,
) -> Result<Json<ConversionResponse>, AppError> {
    approve_or_reject(state, creator_id, conversion_id, "rejected").await
}

async fn approve_or_reject(
    state: AppState,
    creator_id: Uuid,
    conversion_id: Uuid,
    new_status: &str,
) -> Result<Json<ConversionResponse>, AppError> {
    let existing: Option<Conversion> =
        sqlx::query_as("SELECT * FROM conversions WHERE id = $1")
            .bind(conversion_id)
            .fetch_optional(&state.db)
            .await?;

    let existing = existing.ok_or_else(|| AppError::NotFound("Conversion not found".to_string()))?;

    if existing.creator_id != creator_id {
        return Err(AppError::Forbidden);
    }

    let updated: Conversion = sqlx::query_as(
        r#"
        UPDATE conversions
        SET status = $1
        WHERE id = $2
        RETURNING *
        "#,
    )
    .bind(new_status)
    .bind(conversion_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(ConversionResponse::from(updated)))
}

// --- Postback Handler (Public — no auth, called by platforms) ---

pub async fn handle_postback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<PostbackRequest>,
) -> Result<(StatusCode, Json<PostbackResponse>), AppError> {
    // B.4: Rate limiting via Redis
    let ip = crate::routes::redirect::extract_client_ip(&headers);
    let rate_key = format!("ratelimit:postback:{}", ip);
    let count = state.redis.increment(&rate_key, 60).await?;
    if count > 30 {
        return Err(AppError::RateLimited);
    }

    // Validate click_id
    let click_id = Uuid::parse_str(&payload.click_id)
        .map_err(|_| AppError::Validation("Invalid click_id format".to_string()))?;

    // B.5: Validate raw_data size (10KB limit)
    if let Some(ref raw_data) = payload.raw_data {
        let json_str = serde_json::to_string(raw_data)
            .map_err(|e| AppError::Internal(format!("Serialization error: {}", e)))?;
        if json_str.len() > 10_000 {
            return Err(AppError::Validation(
                "raw_data exceeds maximum size of 10KB".to_string(),
            ));
        }
    }

    // Find the redirect event by click_id
    let merchant_link_id: Uuid = sqlx::query_scalar(
        "SELECT merchant_link_id FROM redirect_events WHERE click_id = $1 LIMIT 1",
    )
    .bind(click_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?
    .ok_or_else(|| AppError::NotFound("Invalid click_id: no matching redirect event".to_string()))?;

    // B.2: Check for duplicate conversion with same click_id
    let existing_conversion: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM conversions WHERE click_id = $1 LIMIT 1",
    )
    .bind(click_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

    if existing_conversion.is_some() {
        return Err(AppError::Conflict(
            "Conversion already exists for this click_id".to_string(),
        ));
    }

    // Get the merchant link to find the creator_id
    let merchant_link: Option<MerchantLink> =
        sqlx::query_as("SELECT * FROM merchant_links WHERE id = $1")
            .bind(merchant_link_id)
            .fetch_optional(&state.db)
            .await?;

    let merchant_link = merchant_link
        .ok_or_else(|| AppError::NotFound("Merchant link not found".to_string()))?;

    // Get creator_id from product
    let creator_id: Uuid = sqlx::query_scalar(
        "SELECT creator_id FROM products WHERE id = $1",
    )
    .bind(merchant_link.product_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to fetch product: {}", e)))?;

    // B.3: Create the conversion with postback_at = NOW()
    let conversion: Conversion = sqlx::query_as(
        r#"
        INSERT INTO conversions (click_id, merchant_link_id, creator_id, platform,
            order_id, order_amount, currency, commission, conversion_type,
            product_name, raw_data, postback_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NOW())
        RETURNING *
        "#,
    )
    .bind(click_id)
    .bind(merchant_link_id)
    .bind(creator_id)
    .bind(&payload.platform)
    .bind(&payload.order_id)
    .bind(payload.order_amount)
    .bind(payload.currency.as_deref().unwrap_or("THB"))
    .bind(payload.commission)
    .bind(payload.conversion_type.as_deref().unwrap_or("sale"))
    .bind(&payload.product_name)
    .bind(payload.raw_data.unwrap_or_default())
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.code().map_or(false, |c| c == "23503") {
                tracing::warn!(
                    "Postback FK violation for click_id {}: {}",
                    click_id,
                    db_err.message()
                );
                return AppError::Validation(
                    "Referenced resource (click, merchant link, or creator) no longer exists"
                        .to_string(),
                );
            }
        }
        tracing::error!("Failed to create conversion: {}", e);
        AppError::Internal(format!("Failed to create conversion: {}", e))
    })?;

    Ok((
        StatusCode::CREATED,
        Json(PostbackResponse {
            conversion_id: conversion.id,
            status: conversion.status,
        }),
    ))
}
