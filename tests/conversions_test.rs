mod common;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use backend::middleware::auth::AuthCreator;
use backend::routes::admin_conversions::{
    handle_postback, list_conversions, get_conversion, update_conversion,
    approve_conversion, reject_conversion, ConversionListParams, PostbackRequest, UpdateConversionRequest,
};
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
    let email = format!("conversion-test-{}@trendcart.test", Uuid::new_v4());
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

async fn create_test_click(state: &AppState, merchant_link_id: Uuid) -> Uuid {
    // Simulate a redirect click to create a redirect_events entry with click_id
    let app = backend::routes::router(state.clone());
    let request = Request::builder()
        .method("GET")
        .uri(format!("/redirect?merchant={}", merchant_link_id))
        .header("User-Agent", "TestBot/1.0")
        .header("X-Forwarded-For", "192.168.1.100")
        .body(Body::empty())
        .unwrap();

    let _ = ServiceExt::oneshot(app, request).await.unwrap();

    // Get the click_id from the redirect event
    let click_id: Uuid = sqlx::query_scalar(
        "SELECT click_id FROM redirect_events WHERE merchant_link_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(merchant_link_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    click_id
}

// --- Postback Tests ---

#[tokio::test]
async fn postback_creates_conversion() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;
    let click_id = create_test_click(&state, link_id).await;

    let request = PostbackRequest {
        click_id: click_id.to_string(),
        order_id: Some("SHOPEE-12345".to_string()),
        order_amount: Some(599.00),
        currency: Some("THB".to_string()),
        commission: Some(30.00),
        platform: "shopee".to_string(),
        conversion_type: Some("sale".to_string()),
        product_name: Some("Test Product".to_string()),
        raw_data: None,
    };

    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(request))
        .await
        .unwrap()
        .into_response();

    assert_eq!(response.status(), StatusCode::CREATED);
    let json = body_json(response).await;
    assert_eq!(json["status"], "pending");

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn postback_returns_404_for_invalid_click_id() {
    let state = build_state().await;

    let request = PostbackRequest {
        click_id: Uuid::new_v4().to_string(),
        order_id: None,
        order_amount: None,
        currency: None,
        commission: None,
        platform: "shopee".to_string(),
        conversion_type: None,
        product_name: None,
        raw_data: None,
    };

    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(request))
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn postback_rejects_invalid_uuid_format() {
    let state = build_state().await;

    let request = PostbackRequest {
        click_id: "not-a-uuid".to_string(),
        order_id: None,
        order_amount: None,
        currency: None,
        commission: None,
        platform: "shopee".to_string(),
        conversion_type: None,
        product_name: None,
        raw_data: None,
    };

    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(request))
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// --- Admin Conversion Tests ---

#[tokio::test]
async fn list_conversions_returns_empty_for_new_creator() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let response = list_conversions(
        State(state.clone()),
        AuthCreator(creator_id),
        axum::extract::Query(ConversionListParams {
            page: None,
            limit: None,
            status: None,
            platform: None,
        }),
    )
    .await
    .unwrap()
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["total"], 0);
    assert_eq!(json["items"].as_array().unwrap().len(), 0);

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn list_conversions_returns_created_conversion() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;
    let click_id = create_test_click(&state, link_id).await;

    // Create a conversion via postback
    let postback = PostbackRequest {
        click_id: click_id.to_string(),
        order_id: Some("ORDER-001".to_string()),
        order_amount: Some(299.00),
        currency: Some("THB".to_string()),
        commission: Some(15.00),
        platform: "shopee".to_string(),
        conversion_type: Some("sale".to_string()),
        product_name: Some("Product".to_string()),
        raw_data: None,
    };

    let _ = handle_postback(State(state.clone()), HeaderMap::new(), Json(postback))
        .await
        .unwrap()
        .into_response();

    // List conversions
    let response = list_conversions(
        State(state.clone()),
        AuthCreator(creator_id),
        axum::extract::Query(ConversionListParams {
            page: None,
            limit: None,
            status: None,
            platform: None,
        }),
    )
    .await
    .unwrap()
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["total"], 1);
    assert_eq!(json["items"][0]["platform"], "shopee");
    assert_eq!(json["items"][0]["order_id"], "ORDER-001");

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn approve_conversion_updates_status() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;
    let click_id = create_test_click(&state, link_id).await;

    // Create conversion
    let postback = PostbackRequest {
        click_id: click_id.to_string(),
        order_id: None,
        order_amount: None,
        currency: None,
        commission: None,
        platform: "shopee".to_string(),
        conversion_type: None,
        product_name: None,
        raw_data: None,
    };

    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(postback))
        .await
        .unwrap()
        .into_response();
    let json = body_json(response).await;
    let conversion_id: Uuid = json["conversion_id"].as_str().unwrap().parse().unwrap();

    // Approve
    let response = approve_conversion(
        State(state.clone()),
        AuthCreator(creator_id),
        axum::extract::Path(conversion_id),
    )
    .await
    .unwrap()
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["status"], "approved");

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn reject_conversion_updates_status() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;
    let click_id = create_test_click(&state, link_id).await;

    // Create conversion
    let postback = PostbackRequest {
        click_id: click_id.to_string(),
        order_id: None,
        order_amount: None,
        currency: None,
        commission: None,
        platform: "shopee".to_string(),
        conversion_type: None,
        product_name: None,
        raw_data: None,
    };

    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(postback))
        .await
        .unwrap()
        .into_response();
    let json = body_json(response).await;
    let conversion_id: Uuid = json["conversion_id"].as_str().unwrap().parse().unwrap();

    // Reject
    let response = reject_conversion(
        State(state.clone()),
        AuthCreator(creator_id),
        axum::extract::Path(conversion_id),
    )
    .await
    .unwrap()
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["status"], "rejected");

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn update_conversion_validates_status() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;
    let click_id = create_test_click(&state, link_id).await;

    // Create conversion
    let postback = PostbackRequest {
        click_id: click_id.to_string(),
        order_id: None,
        order_amount: None,
        currency: None,
        commission: None,
        platform: "shopee".to_string(),
        conversion_type: None,
        product_name: None,
        raw_data: None,
    };

    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(postback))
        .await
        .unwrap()
        .into_response();
    let json = body_json(response).await;
    let conversion_id: Uuid = json["conversion_id"].as_str().unwrap().parse().unwrap();

    // Try invalid status
    let response = update_conversion(
        State(state.clone()),
        AuthCreator(creator_id),
        axum::extract::Path(conversion_id),
        Json(UpdateConversionRequest {
            status: "invalid_status".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn get_conversion_returns_404_for_nonexistent() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let response = get_conversion(
        State(state.clone()),
        AuthCreator(creator_id),
        axum::extract::Path(Uuid::new_v4()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn list_conversions_filters_by_status() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;
    let click_id = create_test_click(&state, link_id).await;

    // Create conversion
    let postback = PostbackRequest {
        click_id: click_id.to_string(),
        order_id: None,
        order_amount: None,
        currency: None,
        commission: None,
        platform: "shopee".to_string(),
        conversion_type: None,
        product_name: None,
        raw_data: None,
    };

    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(postback))
        .await
        .unwrap()
        .into_response();
    let json = body_json(response).await;
    let conversion_id: Uuid = json["conversion_id"].as_str().unwrap().parse().unwrap();

    // Approve the conversion
    let _ = approve_conversion(
        State(state.clone()),
        AuthCreator(creator_id),
        axum::extract::Path(conversion_id),
    )
    .await
    .unwrap()
    .into_response();

    // Filter by pending (should be empty)
    let response = list_conversions(
        State(state.clone()),
        AuthCreator(creator_id),
        axum::extract::Query(ConversionListParams {
            page: None,
            limit: None,
            status: Some("pending".to_string()),
            platform: None,
        }),
    )
    .await
    .unwrap()
    .into_response();

    let json = body_json(response).await;
    assert_eq!(json["total"], 0);

    // Filter by approved (should have 1)
    let response = list_conversions(
        State(state.clone()),
        AuthCreator(creator_id),
        axum::extract::Query(ConversionListParams {
            page: None,
            limit: None,
            status: Some("approved".to_string()),
            platform: None,
        }),
    )
    .await
    .unwrap()
    .into_response();

    let json = body_json(response).await;
    assert_eq!(json["total"], 1);

    cleanup_creator(&state.db, creator_id).await;
}

// --- New Phase B Tests ---

#[tokio::test]
async fn postback_duplicate_click_id_returns_409() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;
    let click_id = create_test_click(&state, link_id).await;

    // First postback should succeed
    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(PostbackRequest {
        click_id: click_id.to_string(),
        order_id: None, order_amount: None, currency: None, commission: None,
        platform: "shopee".to_string(), conversion_type: None, product_name: None, raw_data: None,
    }))
    .await
    .unwrap()
    .into_response();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Second postback with same click_id should return 409
    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(PostbackRequest {
        click_id: click_id.to_string(),
        order_id: None, order_amount: None, currency: None, commission: None,
        platform: "shopee".to_string(), conversion_type: None, product_name: None, raw_data: None,
    }))
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn postback_sets_postback_at() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;
    let click_id = create_test_click(&state, link_id).await;

    let request = PostbackRequest {
        click_id: click_id.to_string(),
        order_id: None,
        order_amount: None,
        currency: None,
        commission: None,
        platform: "shopee".to_string(),
        conversion_type: None,
        product_name: None,
        raw_data: None,
    };

    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(request))
        .await
        .unwrap()
        .into_response();
    assert_eq!(response.status(), StatusCode::CREATED);
    let json = body_json(response).await;
    let conversion_id: Uuid = json["conversion_id"].as_str().unwrap().parse().unwrap();

    // Verify postback_at is set (not NULL)
    let postback_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT postback_at FROM conversions WHERE id = $1",
    )
    .bind(conversion_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert!(postback_at.is_some(), "postback_at should not be NULL");

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn postback_raw_data_too_large_returns_400() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;
    let (link_id, _) = create_test_merchant_link(&state, creator_id, product_id).await;
    let click_id = create_test_click(&state, link_id).await;

    // Create a raw_data value > 10KB
    let large_string = "x".repeat(11_000);
    let large_raw_data = serde_json::json!({ "data": large_string });

    let request = PostbackRequest {
        click_id: click_id.to_string(),
        order_id: None,
        order_amount: None,
        currency: None,
        commission: None,
        platform: "shopee".to_string(),
        conversion_type: None,
        product_name: None,
        raw_data: Some(large_raw_data),
    };

    let response = handle_postback(State(state.clone()), HeaderMap::new(), Json(request))
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    cleanup_creator(&state.db, creator_id).await;
}
