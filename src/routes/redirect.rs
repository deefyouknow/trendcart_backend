use std::net::IpAddr;

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, serde::Deserialize)]
pub struct RedirectQuery {
    pub merchant: Uuid,
    pub variant: Option<Uuid>,
}

/// Extract client IP from X-Forwarded-For header, falling back to 0.0.0.0
pub fn extract_client_ip(headers: &HeaderMap) -> IpAddr {
    if let Some(forwarded) = headers.get("X-Forwarded-For") {
        if let Ok(val) = forwarded.to_str() {
            if let Some(first_ip) = val.split(',').next() {
                if let Ok(ip) = first_ip.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
    }
    if let Some(real_ip) = headers.get("X-Real-IP") {
        if let Ok(val) = real_ip.to_str() {
            if let Ok(ip) = val.trim().parse::<IpAddr>() {
                return ip;
            }
        }
    }
    IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0))
}

pub async fn redirect_handler(
    State(state): State<AppState>,
    Query(query): Query<RedirectQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let ip = extract_client_ip(&headers);

    // Rate limiting via Redis INCR + EXPIRE
    let rate_key = format!("ratelimit:redirect:{}", ip);
    let count = state.redis.increment(&rate_key, 60).await?;
    if count > 30 {
        return Err(AppError::RateLimited);
    }

    // Fetch merchant link
    let affiliate_url: Option<String> = sqlx::query_scalar(
        "SELECT affiliate_url FROM merchant_links WHERE id = $1",
    )
    .bind(query.merchant)
    .fetch_optional(&state.db)
    .await?;

    let affiliate_url = match affiliate_url {
        Some(url) => url,
        None => return Err(AppError::NotFound("Merchant link not found".to_string())),
    };

    // Validate variant belongs to merchant link if provided
    if let Some(variant_id) = query.variant {
        let variant_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM product_variants WHERE id = $1 AND merchant_link_id = $2)",
        )
        .bind(variant_id)
        .bind(query.merchant)
        .fetch_one(&state.db)
        .await?;

        if !variant_exists {
            return Err(AppError::NotFound(
                "Variant not found for this merchant link".to_string(),
            ));
        }
    }

    // Buffer click event in Redis (write-behind) — not direct Postgres INSERT
    let user_agent = headers
        .get("User-Agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let event = serde_json::json!({
        "merchant_link_id": query.merchant,
        "variant_id": query.variant,
        "ip": ip.to_string(),
        "user_agent": user_agent,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    state
        .redis
        .push_event("click:pending", &event.to_string())
        .await;

    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, affiliate_url)],
    )
        .into_response())
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/redirect", axum::routing::get(redirect_handler))
}
