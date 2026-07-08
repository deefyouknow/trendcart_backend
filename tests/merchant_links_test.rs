mod common;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use backend::middleware::auth::AuthCreator;
use backend::routes::admin_merchant_links::{
    create, delete_link, get_one, update, verify_price, CreateMerchantLinkRequest,
    CreateVariantRequest, UpdateMerchantLinkRequest, VerifyPriceRequest,
};
use backend::routes::admin_products::{create as create_product, CreateProductRequest};
use backend::cache::RedisCache;
use backend::state::AppState;
use sqlx::PgPool;
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
    let email = format!("merchant-owner-{}@trendcart.test", Uuid::new_v4());
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

async fn body_json(response: Response) -> serde_json::Value {
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

fn default_variant() -> CreateVariantRequest {
    CreateVariantRequest {
        variant_name: "Default".to_string(),
        price: 299.00,
        currency: Some("THB".to_string()),
        stock_status: Some("in_stock".to_string()),
    }
}

// --- Create Merchant Link ---

#[tokio::test]
async fn create_merchant_link_with_variants_returns_201() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;

    let response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "My Shop".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .expect("create merchant link should succeed")
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let json = body_json(response).await;
    assert_eq!(json["platform"], "shopee");
    assert_eq!(json["store_name"], "My Shop");
    assert_eq!(json["affiliate_url"], "https://shopee.co.th/product/123");
    assert_eq!(json["variants"].as_array().unwrap().len(), 1);
    assert_eq!(json["variants"][0]["variant_name"], "Default");
    assert_eq!(json["variants"][0]["price"], 299.00);

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn create_merchant_link_rejects_empty_platform() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;

    let err = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "".to_string(),
            store_name: "My Shop".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .expect_err("empty platform should be rejected");

    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::BAD_REQUEST
    );

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn create_merchant_link_rejects_invalid_platform() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;

    let err = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "amazon".to_string(),
            store_name: "My Shop".to_string(),
            affiliate_url: "https://amazon.com/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .expect_err("invalid platform should be rejected");

    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::BAD_REQUEST
    );

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn create_merchant_link_rejects_empty_variants() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;

    let err = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "My Shop".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![],
        }),
    )
    .await
    .expect_err("empty variants should be rejected");

    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::BAD_REQUEST
    );

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn create_merchant_link_returns_404_for_nonexistent_product() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let err = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(Uuid::new_v4()),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "My Shop".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .expect_err("nonexistent product should 404");

    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::NOT_FOUND
    );

    cleanup_creator(&state.db, creator_id).await;
}

// --- Get One ---

#[tokio::test]
async fn get_one_returns_merchant_link_with_variants() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "lazada".to_string(),
            store_name: "Laz Shop".to_string(),
            affiliate_url: "https://lazada.co.th/product/456".to_string(),
            variants: vec![
                CreateVariantRequest {
                    variant_name: "Small".to_string(),
                    price: 199.00,
                    currency: Some("THB".to_string()),
                    stock_status: Some("in_stock".to_string()),
                },
                CreateVariantRequest {
                    variant_name: "Large".to_string(),
                    price: 399.00,
                    currency: Some("THB".to_string()),
                    stock_status: Some("out_of_stock".to_string()),
                },
            ],
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let link_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let response = get_one(State(state.clone()), AuthCreator(creator_id), Path(link_id))
        .await
        .expect("get_one should succeed")
        .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["platform"], "lazada");
    assert_eq!(json["store_name"], "Laz Shop");
    let variants = json["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 2);

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn get_one_returns_404_for_nonexistent_link() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let err = get_one(State(state.clone()), AuthCreator(creator_id), Path(Uuid::new_v4()))
        .await
        .expect_err("nonexistent link should 404");

    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::NOT_FOUND
    );

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn get_one_returns_403_for_non_owner() {
    let state = build_state().await;
    let owner_id = create_test_creator(&state.db).await;
    let intruder_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, owner_id).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(owner_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "My Shop".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let link_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let err = get_one(State(state.clone()), AuthCreator(intruder_id), Path(link_id))
        .await
        .expect_err("non-owner should be forbidden");

    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::FORBIDDEN
    );

    cleanup_creator(&state.db, owner_id).await;
    cleanup_creator(&state.db, intruder_id).await;
}

// --- Update ---

#[tokio::test]
async fn update_modifies_merchant_link() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "Old Shop".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let link_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let response = update(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(link_id),
        Json(UpdateMerchantLinkRequest {
            platform: Some("lazada".to_string()),
            store_name: Some("New Shop".to_string()),
            affiliate_url: None,
            is_price_estimated: None,
        }),
    )
    .await
    .expect("update should succeed")
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["platform"], "lazada");
    assert_eq!(json["store_name"], "New Shop");

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn update_returns_403_for_non_owner() {
    let state = build_state().await;
    let owner_id = create_test_creator(&state.db).await;
    let intruder_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, owner_id).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(owner_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "Protected Shop".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let link_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let err = update(
        State(state.clone()),
        AuthCreator(intruder_id),
        Path(link_id),
        Json(UpdateMerchantLinkRequest {
            platform: Some("tiktok".to_string()),
            store_name: None,
            affiliate_url: None,
            is_price_estimated: None,
        }),
    )
    .await
    .expect_err("non-owner update should be forbidden");

    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::FORBIDDEN
    );

    cleanup_creator(&state.db, owner_id).await;
    cleanup_creator(&state.db, intruder_id).await;
}

// --- Delete ---

#[tokio::test]
async fn delete_removes_merchant_link() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "To Delete".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let link_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let response = delete_link(State(state.clone()), AuthCreator(creator_id), Path(link_id))
        .await
        .expect("delete should succeed")
        .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);

    // Verify it's gone
    let err = get_one(State(state.clone()), AuthCreator(creator_id), Path(link_id))
        .await
        .expect_err("deleted link should 404");
    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::NOT_FOUND
    );

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn delete_cascades_to_variants() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "Cascade Delete".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let link_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    // Verify variant exists
    let variant_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM product_variants WHERE merchant_link_id = $1",
    )
    .bind(link_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(variant_count, 1);

    // Delete the merchant link
    delete_link(State(state.clone()), AuthCreator(creator_id), Path(link_id))
        .await
        .unwrap();

    // Verify variant is also deleted
    let variant_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM product_variants WHERE merchant_link_id = $1",
    )
    .bind(link_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(variant_count, 0);

    cleanup_creator(&state.db, creator_id).await;
}

// --- Verify Price ---

#[tokio::test]
async fn verify_price_sets_price_checked_at() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "Price Check".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let link_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();
    assert!(created_json["price_checked_at"].is_null());

    let response = verify_price(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(link_id),
        Json(VerifyPriceRequest {
            is_price_estimated: Some(false),
        }),
    )
    .await
    .expect("verify_price should succeed")
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["is_price_estimated"], false);
    assert!(!json["price_checked_at"].is_null());

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn verify_price_returns_403_for_non_owner() {
    let state = build_state().await;
    let owner_id = create_test_creator(&state.db).await;
    let intruder_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, owner_id).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(owner_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "Protected Price".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let link_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let err = verify_price(
        State(state.clone()),
        AuthCreator(intruder_id),
        Path(link_id),
        Json(VerifyPriceRequest {
            is_price_estimated: Some(false),
        }),
    )
    .await
    .expect_err("non-owner verify should be forbidden");

    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::FORBIDDEN
    );

    cleanup_creator(&state.db, owner_id).await;
    cleanup_creator(&state.db, intruder_id).await;
}

// --- Product Detail with Merchant Links ---

#[tokio::test]
async fn admin_get_one_includes_merchant_links() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let product_id = create_test_product(&state, creator_id).await;

    // Add a merchant link
    create(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(CreateMerchantLinkRequest {
            platform: "shopee".to_string(),
            store_name: "Test Shop".to_string(),
            affiliate_url: "https://shopee.co.th/product/123".to_string(),
            variants: vec![default_variant()],
        }),
    )
    .await
    .unwrap();

    // Get product detail
    let response = backend::routes::admin_products::get_one(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
    )
    .await
    .expect("get_one should succeed")
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let json = body_json(response).await;
    let links = json["merchant_links"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["platform"], "shopee");
    assert_eq!(links[0]["variants"].as_array().unwrap().len(), 1);

    cleanup_creator(&state.db, creator_id).await;
}
