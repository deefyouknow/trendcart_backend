mod common;

use axum::extract::FromRequestParts;
use axum::extract::State;
use axum::http::Request;
use axum::response::{IntoResponse, Response};
use axum::Json;
use backend::middleware::auth::AuthCreator;
use backend::routes::auth::{login, register, LoginRequest, RegisterRequest};
use backend::cache::RedisCache;
use backend::state::AppState;
use uuid::Uuid;

async fn build_state() -> AppState {
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    AppState {
        db: common::test_pool().await,
        jwt_secret: "test_secret".to_string(),
        redis: RedisCache::new(&redis_url).await.expect("redis should connect"),
        job_sender: None,
    }
}

async fn body_json(response: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// --- AuthCreator extractor tests ---

#[tokio::test]
async fn auth_creator_extracts_uuid_from_valid_bearer_token() {
    let state = build_state().await;
    let creator_id = Uuid::new_v4();
    let token = backend::jwt::create_token(creator_id, &state.jwt_secret).unwrap();

    let request = Request::builder()
        .header("Authorization", format!("Bearer {}", token))
        .body(())
        .unwrap();
    let (mut parts, _) = request.into_parts();

    let AuthCreator(extracted_id) = AuthCreator::from_request_parts(&mut parts, &state)
        .await
        .expect("valid token should extract successfully");

    assert_eq!(extracted_id, creator_id);
}

#[tokio::test]
async fn auth_creator_rejects_missing_authorization_header() {
    let state = build_state().await;
    let request = Request::builder().body(()).unwrap();
    let (mut parts, _) = request.into_parts();

    let result = AuthCreator::from_request_parts(&mut parts, &state).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn auth_creator_rejects_malformed_header_without_bearer_prefix() {
    let state = build_state().await;
    let request = Request::builder()
        .header("Authorization", "Token abc123")
        .body(())
        .unwrap();
    let (mut parts, _) = request.into_parts();

    let result = AuthCreator::from_request_parts(&mut parts, &state).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn auth_creator_rejects_expired_or_invalid_token() {
    let state = build_state().await;
    let request = Request::builder()
        .header("Authorization", "Bearer not.a.valid.jwt")
        .body(())
        .unwrap();
    let (mut parts, _) = request.into_parts();

    let result = AuthCreator::from_request_parts(&mut parts, &state).await;
    assert!(result.is_err());
}

// --- Register tests ---

#[tokio::test]
async fn register_creates_creator_and_returns_201() {
    let state = build_state().await;
    let email = format!("register-{}@trendcart.test", Uuid::new_v4());

    let payload = RegisterRequest {
        email: email.clone(),
        password: "password123".to_string(),
        display_name: "Register Test".to_string(),
    };

    let response = register(State(state.clone()), Json(payload))
        .await
        .expect("registration should succeed")
        .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let json = body_json(response).await;
    assert_eq!(json["email"], email);
    assert_eq!(json["display_name"], "Register Test");
    assert!(json.get("id").is_some());

    sqlx::query("DELETE FROM creators WHERE email = $1")
        .bind(&email)
        .execute(&state.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn register_rejects_duplicate_email_with_409() {
    let state = build_state().await;
    let email = format!("dup-{}@trendcart.test", Uuid::new_v4());

    register(
        State(state.clone()),
        Json(RegisterRequest {
            email: email.clone(),
            password: "password123".to_string(),
            display_name: "First".to_string(),
        }),
    )
    .await
    .expect("first registration should succeed");

    let err = register(
        State(state.clone()),
        Json(RegisterRequest {
            email: email.clone(),
            password: "password123".to_string(),
            display_name: "Second".to_string(),
        }),
    )
    .await
    .expect_err("duplicate registration should fail");

    let response = err.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let json = body_json(response).await;
    assert_eq!(json["error"], "Email already registered");
    assert_eq!(json["code"], "CONFLICT");

    sqlx::query("DELETE FROM creators WHERE email = $1")
        .bind(&email)
        .execute(&state.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn register_rejects_short_password_with_400() {
    let state = build_state().await;
    let email = format!("shortpw-{}@trendcart.test", Uuid::new_v4());

    let err = register(
        State(state),
        Json(RegisterRequest {
            email,
            password: "short".to_string(),
            display_name: "Short Password".to_string(),
        }),
    )
    .await
    .expect_err("short password should be rejected");

    let response = err.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert_eq!(json["code"], "VALIDATION_ERROR");
}

// --- Login tests ---

#[tokio::test]
async fn login_returns_token_and_creator_for_valid_credentials() {
    let state = build_state().await;
    let email = format!("login-{}@trendcart.test", Uuid::new_v4());

    register(
        State(state.clone()),
        Json(RegisterRequest {
            email: email.clone(),
            password: "password123".to_string(),
            display_name: "Login Test".to_string(),
        }),
    )
    .await
    .expect("setup registration should succeed");

    let response = login(
        State(state.clone()),
        Json(LoginRequest {
            email: email.clone(),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect("login should succeed")
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let json = body_json(response).await;
    assert!(!json["token"].as_str().unwrap().is_empty());
    assert_eq!(json["creator"]["email"], email);

    sqlx::query("DELETE FROM creators WHERE email = $1")
        .bind(&email)
        .execute(&state.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn login_rejects_wrong_password_with_401() {
    let state = build_state().await;
    let email = format!("wrongpw-{}@trendcart.test", Uuid::new_v4());

    register(
        State(state.clone()),
        Json(RegisterRequest {
            email: email.clone(),
            password: "password123".to_string(),
            display_name: "Wrong Password Test".to_string(),
        }),
    )
    .await
    .expect("setup registration should succeed");

    let err = login(
        State(state.clone()),
        Json(LoginRequest {
            email: email.clone(),
            password: "totally_wrong_password".to_string(),
        }),
    )
    .await
    .expect_err("wrong password should be rejected");

    let response = err.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    let json = body_json(response).await;
    assert_eq!(json["code"], "UNAUTHORIZED");

    sqlx::query("DELETE FROM creators WHERE email = $1")
        .bind(&email)
        .execute(&state.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn login_rejects_unknown_email_with_401() {
    let state = build_state().await;
    let err = login(
        State(state),
        Json(LoginRequest {
            email: format!("nonexistent-{}@trendcart.test", Uuid::new_v4()),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect_err("unknown email should be rejected");

    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::UNAUTHORIZED
    );
}
