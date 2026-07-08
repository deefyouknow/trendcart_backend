mod common;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use backend::middleware::auth::AuthCreator;
use backend::routes::admin_products::{
    create, delete, get_one, list, update, CreateProductRequest, ListParams, UpdateProductRequest,
};
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
    let email = format!("product-owner-{}@trendcart.test", Uuid::new_v4());
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

// --- Admin List ---

#[tokio::test]
async fn list_returns_only_products_owned_by_creator() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;
    let other_creator_id = create_test_creator(&state.db).await;

    sqlx::query("INSERT INTO products (creator_id, title, category) VALUES ($1, $2, $3)")
        .bind(creator_id)
        .bind("My Product")
        .bind("electronics")
        .execute(&state.db)
        .await
        .unwrap();

    sqlx::query("INSERT INTO products (creator_id, title) VALUES ($1, $2)")
        .bind(other_creator_id)
        .bind("Someone Else's Product")
        .execute(&state.db)
        .await
        .unwrap();

    let response = list(
        State(state.clone()),
        AuthCreator(creator_id),
        Query(ListParams {
            page: None,
            limit: None,
            category: None,
            search: None,
        }),
    )
    .await
    .expect("list should succeed")
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let json = body_json(response).await;
    let products = json["products"].as_array().unwrap();
    assert_eq!(products.len(), 1);
    assert_eq!(products[0]["title"], "My Product");
    assert_eq!(products[0]["merchant_links_count"], 0);
    assert_eq!(json["total"], 1);

    cleanup_creator(&state.db, creator_id).await;
    cleanup_creator(&state.db, other_creator_id).await;
}

// --- Admin Create ---

#[tokio::test]
async fn create_persists_product_and_returns_201() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Json(CreateProductRequest {
            title: "New Gadget".to_string(),
            description: Some("A cool gadget".to_string()),
            category: Some("electronics".to_string()),
            images: None,
        }),
    )
    .await
    .expect("create should succeed")
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let json = body_json(response).await;
    assert_eq!(json["title"], "New Gadget");
    assert_eq!(json["category"], "electronics");
    assert_eq!(json["images"], serde_json::json!([]));

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn create_rejects_empty_title_with_400() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let err = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Json(CreateProductRequest {
            title: "".to_string(),
            description: None,
            category: None,
            images: None,
        }),
    )
    .await
    .expect_err("empty title should be rejected");

    let response = err.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);

    cleanup_creator(&state.db, creator_id).await;
}

// --- Admin Get One ---

#[tokio::test]
async fn get_one_returns_product_detail_for_owner() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Json(CreateProductRequest {
            title: "Detail Product".to_string(),
            description: None,
            category: None,
            images: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let product_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let response = get_one(State(state.clone()), AuthCreator(creator_id), Path(product_id))
        .await
        .expect("get_one should succeed")
        .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["id"], product_id.to_string());
    assert_eq!(json["merchant_links"], serde_json::json!([]));

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn get_one_returns_404_for_nonexistent_product() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let err = get_one(State(state.clone()), AuthCreator(creator_id), Path(Uuid::new_v4()))
        .await
        .expect_err("nonexistent product should 404");

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

    let create_response = create(
        State(state.clone()),
        AuthCreator(owner_id),
        Json(CreateProductRequest {
            title: "Owned Product".to_string(),
            description: None,
            category: None,
            images: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let product_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let err = get_one(State(state.clone()), AuthCreator(intruder_id), Path(product_id))
        .await
        .expect_err("non-owner should be forbidden");

    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::FORBIDDEN
    );

    cleanup_creator(&state.db, owner_id).await;
    cleanup_creator(&state.db, intruder_id).await;
}

// --- Admin Update ---

#[tokio::test]
async fn update_modifies_owned_product() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Json(CreateProductRequest {
            title: "Old Title".to_string(),
            description: None,
            category: None,
            images: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let product_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let response = update(
        State(state.clone()),
        AuthCreator(creator_id),
        Path(product_id),
        Json(UpdateProductRequest {
            title: Some("New Title".to_string()),
            description: None,
            category: None,
            images: None,
        }),
    )
    .await
    .expect("update should succeed")
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["title"], "New Title");

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn update_returns_403_for_non_owner() {
    let state = build_state().await;
    let owner_id = create_test_creator(&state.db).await;
    let intruder_id = create_test_creator(&state.db).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(owner_id),
        Json(CreateProductRequest {
            title: "Protected Product".to_string(),
            description: None,
            category: None,
            images: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let product_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let err = update(
        State(state.clone()),
        AuthCreator(intruder_id),
        Path(product_id),
        Json(UpdateProductRequest {
            title: Some("Hijacked".to_string()),
            description: None,
            category: None,
            images: None,
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

// --- Admin Soft Delete ---

#[tokio::test]
async fn delete_soft_deletes_owned_product() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Json(CreateProductRequest {
            title: "To Be Deleted".to_string(),
            description: None,
            category: None,
            images: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let product_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let response = delete(State(state.clone()), AuthCreator(creator_id), Path(product_id))
        .await
        .expect("delete should succeed")
        .into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let deleted_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT deleted_at FROM products WHERE id = $1")
            .bind(product_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert!(deleted_at.is_some());

    let err = get_one(State(state.clone()), AuthCreator(creator_id), Path(product_id))
        .await
        .expect_err("soft-deleted product should 404 on subsequent get");
    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::NOT_FOUND
    );

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn delete_returns_404_for_nonexistent_product() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let err = delete(State(state.clone()), AuthCreator(creator_id), Path(Uuid::new_v4()))
        .await
        .expect_err("nonexistent product delete should 404");
    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::NOT_FOUND
    );

    cleanup_creator(&state.db, creator_id).await;
}

// --- Store Products ---

#[tokio::test]
async fn store_list_excludes_soft_deleted_and_shows_null_price() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Json(CreateProductRequest {
            title: "Storefront Item".to_string(),
            description: None,
            category: Some("gadgets".to_string()),
            images: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let product_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let deleted_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Json(CreateProductRequest {
            title: "Deleted Item".to_string(),
            description: None,
            category: Some("gadgets".to_string()),
            images: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let deleted_json = body_json(deleted_response).await;
    let deleted_id: Uuid = deleted_json["id"].as_str().unwrap().parse().unwrap();
    delete(State(state.clone()), AuthCreator(creator_id), Path(deleted_id))
        .await
        .unwrap();

    let response = backend::routes::store_products::list(
        State(state.clone()),
        Query(backend::routes::store_products::StoreListParams {
            page: None,
            limit: None,
            category: Some("gadgets".to_string()),
            search: None,
        }),
    )
    .await
    .expect("store list should succeed")
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let json = body_json(response).await;
    let products = json["products"].as_array().unwrap();
    assert_eq!(products.len(), 1);
    assert_eq!(products[0]["id"], product_id.to_string());
    assert!(products[0]["min_price"].is_null());
    assert_eq!(products[0]["platforms"], serde_json::json!([]));

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn store_get_one_returns_public_detail() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Json(CreateProductRequest {
            title: "Public Detail Item".to_string(),
            description: Some("Nice item".to_string()),
            category: None,
            images: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let product_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();

    let response = backend::routes::store_products::get_one(State(state.clone()), Path(product_id))
        .await
        .expect("store get_one should succeed")
        .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["title"], "Public Detail Item");
    assert_eq!(json["merchant_links"], serde_json::json!([]));

    cleanup_creator(&state.db, creator_id).await;
}

#[tokio::test]
async fn store_get_one_returns_404_for_deleted_product() {
    let state = build_state().await;
    let creator_id = create_test_creator(&state.db).await;

    let create_response = create(
        State(state.clone()),
        AuthCreator(creator_id),
        Json(CreateProductRequest {
            title: "Will Be Deleted".to_string(),
            description: None,
            category: None,
            images: None,
        }),
    )
    .await
    .unwrap()
    .into_response();
    let created_json = body_json(create_response).await;
    let product_id: Uuid = created_json["id"].as_str().unwrap().parse().unwrap();
    delete(State(state.clone()), AuthCreator(creator_id), Path(product_id))
        .await
        .unwrap();

    let err = backend::routes::store_products::get_one(State(state.clone()), Path(product_id))
        .await
        .expect_err("deleted product should 404 on store detail");
    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::NOT_FOUND
    );

    cleanup_creator(&state.db, creator_id).await;
}

// --- Full Router Tests ---

#[tokio::test]
async fn full_router_serves_store_products_list_endpoint() {
    let state = build_state().await;
    let app = backend::routes::router(state);

    let request = axum::http::Request::builder()
        .method("GET")
        .uri("/api/store/products")
        .body(axum::body::Body::empty())
        .unwrap();

    let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn full_router_returns_404_for_unknown_route() {
    let state = build_state().await;
    let app = backend::routes::router(state);

    let request = axum::http::Request::builder()
        .method("GET")
        .uri("/api/does-not-exist")
        .body(axum::body::Body::empty())
        .unwrap();

    let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}
