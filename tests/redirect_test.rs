mod common;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use backend::middleware::auth::AuthCreator;
use backend::routes::admin_merchant_links::{
    create as create_merchant_link, CreateMerchantLinkRequest, CreateVariantRequest,
};
use backend::routes::admin_products::{create as create_product, CreateProductRequest};
use backend::cache::RedisCache;
use backend::routes::redirect::RedirectQuery;
use backend::state::AppState;
use sqlx::PgPool;
use tower::ServiceExt;
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

async fn create_test_creator(pool: &PgPool) -> Uuid {
    let email = format!("redirect-test-{}@trendcart.test", Uuid::new_v4());
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO creators (email, password_hash, display_name) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&email)
    .bind("hash")
    .bind("Test Creator")
    .fetch_one(pool)
    .await
    .unwrap();
    id
}

async fn cleanup_creator(pool: &PgPool, creator_id: Uuid) {
    sqlx::query("DELETE FROM creators WHERE id = $1")
        .bind(creator_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn create_test_product(state: &AppState, creator_id: Uuid) -> Uuid {
    let response = create_product(
        State(state.clone()),
        AuthCreator(creator_id),
        Json(CreateProductRequest {
            title: "Test Product".to_string(),
            description: None,
            category: None,
            images: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let json = body_json(response).await;
    json["id"].as_str().unwrap().parse().unwrap()
}

async fn create_test_merchant_link(state: &AppState, creator_id: Uuid, product_id: Uuid) -> (Uuid, Uuid) {
    let response = create_merchant_link(
        State(state.clone()),
        AuthCreator(creator_id),
        axum::extract::Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "Test Shop".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![CreateVariantRequest {
                variant_name: "Default".to_string(),
                price: 299.00,
                currency: Some("THB".to_string()),
                stock_status: Some("in_stock".to_string()),
            }],
        }),
    )
    .await
    .unwrap()
    .into_response();
    let json = body_json(response).await;
    let link_id: Uuid = json["id"].as_str().unwrap().parse().unwrap();
    let variant_id: Uuid = json["variants"][0]["id"].as_str().unwrap().parse().unwrap();
    (link_id, variant_id)
}

// --- Redirect Tests ---

#[tokio::test]
async fn redirect_returns_302_with_location_header() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;

    let app = backend::routes::router(state.clone());
    let request = Request::builder()
        .method("GET")
        .uri(format!("/redirect?merchant={}", link_id))
        .header("User-Agent", "TestBot/1.0")
        .header("X-Forwarded-For", "192.168.1.100")
        .body(Body::empty())
        .unwrap();

    let response = ServiceExt::oneshot(app, request).await.unwrap();

    assert_eq!(response.status(), StatusCode::FOUND);
    let location = response.headers().get("Location").unwrap().to_str().unwrap();
    assert_eq!(location, "https://shopee.co.th/product/123");

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn redirect_logs_click_event() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, variant_id) = create_test_merchant_link(&state, creator_id, product_id).await;

    let app = backend::routes::router(state.clone());
    let request = Request::builder()
        .method("GET")
        .uri(format!("/redirect?merchant={}&variant={}", link_id, variant_id))
        .header("User-Agent", "Mozilla/5.0")
        .header("X-Forwarded-For", "10.0.0.1")
        .body(Body::empty())
        .unwrap();

    let _ = ServiceExt::oneshot(app, request).await.unwrap();

    // Verify click event was logged
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM redirect_events WHERE merchant_link_id = $1",
    )
    .bind(link_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(event_count, 1);

    // Verify variant_id is stored
    let event: (Option<Uuid>,) = sqlx::query_as(
        "SELECT variant_id FROM redirect_events WHERE merchant_link_id = $1",
    )
    .bind(link_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(event.0, Some(variant_id));

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn redirect_returns_404_for_nonexistent_merchant_link() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let app = backend::routes::router(state.clone());
    let fake_id = Uuid::new_v4();
    let request = Request::builder()
        .method("GET")
        .uri(format!("/redirect?merchant={}", fake_id))
        .header("X-Forwarded-For", "10.0.0.2")
        .body(Body::empty())
        .unwrap();

    let response = ServiceExt::oneshot(app, request).await.unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = body_json(response).await;
    assert_eq!(json["error"], "Merchant link not found");

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn redirect_returns_404_for_invalid_variant() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;

    let app = backend::routes::router(state.clone());
    let fake_variant = Uuid::new_v4();
    let request = Request::builder()
        .method("GET")
        .uri(format!("/redirect?merchant={}&variant={}", link_id, fake_variant))
        .header("X-Forwarded-For", "10.0.0.3")
        .body(Body::empty())
        .unwrap();

    let response = ServiceExt::oneshot(app, request).await.unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = body_json(response).await;
    assert_eq!(json["error"], "Variant not found for this merchant link");

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn redirect_works_without_variant() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;

    let app = backend::routes::router(state.clone());
    let request = Request::builder()
        .method("GET")
        .uri(format!("/redirect?merchant={}", link_id))
        .header("X-Forwarded-For", "10.0.0.4")
        .body(Body::empty())
        .unwrap();

    let response = ServiceExt::oneshot(app, request).await.unwrap();

    assert_eq!(response.status(), StatusCode::FOUND);

    // Verify variant_id is NULL in event
    let event: (Option<Uuid>,) = sqlx::query_as(
        "SELECT variant_id FROM redirect_events WHERE merchant_link_id = $1",
    )
    .bind(link_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert!(event.0.is_none());

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn rate_limit_blocks_after_max_requests() {
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let state = AppState {
        db: common::test_pool().await,
        jwt_secret: "test_secret".to_string(),
        redis: RedisCache::new(&redis_url).await.expect("redis should connect"),
        job_sender: None,
    };
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;

    let app = backend::routes::router(state.clone());

    // Send 5 requests from same IP (should succeed)
    for i in 0..5 {
        let request = Request::builder()
            .method("GET")
            .uri(format!("/redirect?merchant={}", link_id))
            .header("X-Forwarded-For", "10.0.0.50")
            .body(Body::empty())
            .unwrap();

        let response = ServiceExt::oneshot(app.clone(), request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FOUND,
            "Request {} should succeed",
            i + 1
        );
    }

    // 6th request from same IP should be rate limited
    let request = Request::builder()
        .method("GET")
        .uri(format!("/redirect?merchant={}", link_id))
        .header("X-Forwarded-For", "10.0.0.50")
        .body(Body::empty())
        .unwrap();

    let response = ServiceExt::oneshot(app, request).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let json = body_json(response).await;
    assert_eq!(
        json["error"],
        "Rate limit exceeded. Please try again later."
    );

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn redirect_logs_ip_address() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;

    let app = backend::routes::router(state.clone());
    let request = Request::builder()
        .method("GET")
        .uri(format!("/redirect?merchant={}", link_id))
        .header("X-Forwarded-For", "172.16.0.1")
        .body(Body::empty())
        .unwrap();

    let _ = ServiceExt::oneshot(app, request).await.unwrap();

    // Verify IP address is logged
    let event: (Option<String>,) = sqlx::query_as(
        "SELECT ip_address::text FROM redirect_events WHERE merchant_link_id = $1",
    )
    .bind(link_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert!(event.0.unwrap().contains("172.16.0.1"));

    cleanup_creator(&state.db, creator_id).await;
}
