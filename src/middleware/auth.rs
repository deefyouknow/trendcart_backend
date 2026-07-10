use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::error::AppError;
use crate::jwt;
use crate::state::AppState;

pub struct AuthCreator(pub Uuid);

impl FromRequestParts<AppState> for AuthCreator {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Try cookie first, then fall back to Authorization header
        let token = extract_token_from_cookie(parts)
            .or_else(|| extract_token_from_header(parts))
            .ok_or(AppError::Unauthorized)?;

        let claims =
            jwt::verify_token(&token, &state.jwt_secret).map_err(|_| AppError::Unauthorized)?;

        let creator_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

        // Guard: ตรวจว่า creator ยังอยู่ใน DB (ป้องกัน 23503 จาก token ของ user ที่ถูกลบไปแล้ว)
        let creator_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM creators WHERE id = $1)",
        )
        .bind(creator_id)
        .fetch_one(&state.db)
        .await
        .map_err(|_| AppError::Unauthorized)?;

        if !creator_exists {
            return Err(AppError::Unauthorized);
        }

        Ok(AuthCreator(creator_id))
    }
}

/// Extract JWT from `tc_token` cookie.
fn extract_token_from_cookie(parts: &Parts) -> Option<String> {
    let cookie_header = parts.headers.get(axum::http::header::COOKIE)?;
    let cookie_str = cookie_header.to_str().ok()?;

    for part in cookie_str.split(';') {
        let mut kv = part.trim().splitn(2, '=');
        let name = kv.next()?.trim();
        let value = kv.next()?.trim();
        if name == "tc_token" {
            return Some(value.to_string());
        }
    }
    None
}

/// Extract JWT from `Authorization: Bearer <token>` header.
fn extract_token_from_header(parts: &Parts) -> Option<String> {
    let header_value = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;

    header_value.strip_prefix("Bearer ").map(String::from)
}
