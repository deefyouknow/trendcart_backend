use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::error::AppError;
use crate::state::AppState;

/// GET /healthz — lightweight liveness probe.
/// Checks DB pool + Redis; returns 200 if both respond, 500 otherwise.
pub async fn healthz(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    // Check DB
    sqlx::query("SELECT 1")
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Internal(format!("DB unhealthy: {}", e)))?;

    // Check Redis
    state
        .redis
        .ping()
        .await
        .map_err(|e| AppError::Internal(format!("Redis unhealthy: {}", e)))?;

    Ok(Json(json!({ "status": "ok" })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn healthz_returns_ok_json() {
        // This test requires a live DB + Redis; skip in CI if unavailable.
        dotenvy::dotenv().ok();
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

        let pool = crate::db::create_pool(&db_url)
            .await
            .expect("DB pool");
        let redis = crate::cache::RedisCache::new(&redis_url)
            .await
            .expect("Redis");

        let state = AppState {
            db: pool,
            jwt_secret: "test".into(),
            redis,
            job_sender: None,
        };

        let resp = healthz(State(state)).await.unwrap().into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["status"], "ok");
    }
}
