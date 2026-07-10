use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Validation(String),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Forbidden")]
    Forbidden,
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Internal(String),
    #[error("Rate limit exceeded. Please try again later.")]
    RateLimited,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, "VALIDATION_ERROR", msg),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "Unauthorized".to_string(),
            ),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN", "Forbidden".to_string()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", msg),
            AppError::Internal(msg) => {
                tracing::error!("internal error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    "Internal server error".to_string(),
                )
            }
            AppError::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "RATE_LIMITED",
                "Rate limit exceeded. Please try again later.".to_string(),
            ),
        };
        let body = Json(serde_json::json!({ "error": message, "code": code }));
        (status, body).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => AppError::NotFound("Resource not found".to_string()),
            sqlx::Error::Database(ref db_err) => {
                if let Some(code) = db_err.code() {
                    match code.as_ref() {
                        "23505" => {
                            // Unique violation
                            AppError::Conflict("Resource already exists".to_string())
                        }
                        "23503" => {
                            // Foreign key violation — referenced data doesn't exist
                            tracing::warn!(
                                "Foreign key violation: {}",
                                db_err.message()
                            );
                            AppError::Validation(
                                "Referenced resource does not exist or has been deleted".to_string(),
                            )
                        }
                        "23502" => {
                            // Not null violation
                            AppError::Validation(format!(
                                "Required field is missing: {}",
                                db_err.message()
                            ))
                        }
                        "23514" => {
                            // Check constraint violation
                            AppError::Validation(format!(
                                "Data validation failed: {}",
                                db_err.message()
                            ))
                        }
                        _ => {
                            tracing::error!("Unhandled database error code {}: {}", code, db_err.message());
                            AppError::Internal(format!("Database error: {}", code))
                        }
                    }
                } else {
                    AppError::Internal(format!("Database error: {}", db_err))
                }
            }
            sqlx::Error::PoolTimedOut => {
                tracing::error!("Connection pool timed out");
                AppError::Internal("Server is busy, please try again".to_string())
            }
            _ => AppError::Internal(format!("Database error: {}", err)),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Internal(format!("JSON error: {}", err))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::Internal(format!("HTTP client error: {}", err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn validation_error_returns_400_with_code() {
        let response = AppError::Validation("title is required".to_string()).into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = body_json(response).await;
        assert_eq!(json["error"], "title is required");
        assert_eq!(json["code"], "VALIDATION_ERROR");
    }

    #[tokio::test]
    async fn unauthorized_error_returns_401_with_code() {
        let response = AppError::Unauthorized.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let json = body_json(response).await;
        assert_eq!(json["code"], "UNAUTHORIZED");
    }

    #[tokio::test]
    async fn forbidden_error_returns_403_with_code() {
        let response = AppError::Forbidden.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = body_json(response).await;
        assert_eq!(json["code"], "FORBIDDEN");
    }

    #[tokio::test]
    async fn not_found_error_returns_404_with_code() {
        let response = AppError::NotFound("Product not found".to_string()).into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = body_json(response).await;
        assert_eq!(json["error"], "Product not found");
        assert_eq!(json["code"], "NOT_FOUND");
    }

    #[tokio::test]
    async fn conflict_error_returns_409_with_code() {
        let response = AppError::Conflict("Email already registered".to_string()).into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let json = body_json(response).await;
        assert_eq!(json["error"], "Email already registered");
        assert_eq!(json["code"], "CONFLICT");
    }

    #[tokio::test]
    async fn internal_error_returns_500_and_hides_real_message() {
        let response = AppError::Internal("leaked db secret detail".to_string()).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = body_json(response).await;
        assert_eq!(json["error"], "Internal server error");
        assert_eq!(json["code"], "INTERNAL_ERROR");
    }

    #[test]
    fn sqlx_row_not_found_converts_to_not_found() {
        let sqlx_err = sqlx::Error::RowNotFound;
        let app_err: AppError = sqlx_err.into();
        assert!(matches!(app_err, AppError::NotFound(_)));
    }

    #[test]
    fn sqlx_pool_timed_out_converts_to_internal() {
        let sqlx_err = sqlx::Error::PoolTimedOut;
        let app_err: AppError = sqlx_err.into();
        assert!(matches!(app_err, AppError::Internal(_)));
    }
}
