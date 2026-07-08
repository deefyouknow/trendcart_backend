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
        let header_value = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(AppError::Unauthorized)?;

        let token = header_value
            .strip_prefix("Bearer ")
            .ok_or(AppError::Unauthorized)?;

        let claims =
            jwt::verify_token(token, &state.jwt_secret).map_err(|_| AppError::Unauthorized)?;

        let creator_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

        Ok(AuthCreator(creator_id))
    }
}
