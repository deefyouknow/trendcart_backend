use std::path::PathBuf;

use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use futures_util::StreamExt;
use tower_http::limit::RequestBodyLimitLayer;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::auth::AuthCreator;
use crate::state::AppState;

// Maximum file size: 10MB
const MAX_FILE_SIZE: usize = 10 * 1024 * 1024;

// Allowed MIME types
const ALLOWED_TYPES: &[&str] = &["image/jpeg", "image/png", "image/heic"];

// Maximum width for resize
const MAX_WIDTH: u32 = 1080;

// Maximum height for resize (to prevent extremely tall images)
const MAX_HEIGHT: u32 = 1080;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/upload", post(upload_image))
        .layer(RequestBodyLimitLayer::new(MAX_FILE_SIZE + 1024)) // 10MB + buffer for multipart headers
}

#[derive(serde::Serialize)]
pub struct UploadResponse {
    path: String,
    width: u32,
    height: u32,
}

async fn upload_image(
    State(_state): State<AppState>,
    AuthCreator(_creator_id): AuthCreator,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<UploadResponse>), AppError> {
    // Extract file from multipart (async)
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::Validation(format!("Failed to read multipart: {}", e)))?
        .ok_or_else(|| AppError::Validation("No file provided".to_string()))?;

    // Get filename and content type
    let filename = field
        .file_name()
        .ok_or_else(|| AppError::Validation("No filename provided".to_string()))?
        .to_string();

    let content_type = field
        .content_type()
        .ok_or_else(|| AppError::Validation("No content type provided".to_string()))?
        .to_string();

    // Validate content type
    if !ALLOWED_TYPES.contains(&content_type.as_str()) {
        return Err(AppError::Validation(format!(
            "Invalid file type: {}. Allowed types: jpg, png, heic",
            content_type
        )));
    }

    // Read file data
    let data = field
        .bytes()
        .await
        .map_err(|e| AppError::Validation(format!("Failed to read file: {}", e)))?;

    // Validate file size
    if data.len() > MAX_FILE_SIZE {
        return Err(AppError::Validation(format!(
            "File too large: {} bytes. Maximum size: {} bytes",
            data.len(),
            MAX_FILE_SIZE
        )));
    }

    // Move all blocking work (fs I/O + image processing) to a dedicated thread pool
    // so we don't block the async runtime and exhaust the DB connection pool
    let upload_result = tokio::task::spawn_blocking(move || {
        // Generate unique ID for this upload
        let upload_id = Uuid::new_v4();
        let upload_dir = PathBuf::from("uploads/products").join(upload_id.to_string());

        // Create upload directory
        std::fs::create_dir_all(&upload_dir)
            .map_err(|e| AppError::Internal(format!("Failed to create upload directory: {}", e)))?;

        // Decode image
        let img = image::load_from_memory(&data)
            .map_err(|e| AppError::Validation(format!("Invalid image: {}", e)))?;

        // Resize and crop to fill exact bounds (1:1 square)
        let resized = img.resize_to_fill(MAX_WIDTH, MAX_HEIGHT, image::imageops::FilterType::Lanczos3);

        // Get dimensions
        let width = resized.width();
        let height = resized.height();

        // Convert to WebP
        let webp_data = encode_webp(&resized)
            .map_err(|e| AppError::Internal(format!("Failed to encode WebP: {}", e)))?;

        // Save WebP file
        let webp_path = upload_dir.join("optimized.webp");
        std::fs::write(&webp_path, &webp_data)
            .map_err(|e| AppError::Internal(format!("Failed to save WebP: {}", e)))?;

        Ok::<_, AppError>((upload_id, width, height))
    })
    .await
    .map_err(|e| AppError::Internal(format!("Upload task failed: {}", e)))?;

    let (upload_id, width, height) = upload_result?;

    // Return path and dimensions
    let path = format!("/uploads/products/{}/optimized.webp", upload_id);

    tracing::info!(
        "Image uploaded: {} ({}x{}) -> WebP",
        filename,
        width,
        height
    );

    Ok((
        StatusCode::CREATED,
        Json(UploadResponse {
            path,
            width,
            height,
        }),
    ))
}

fn encode_webp(img: &image::DynamicImage) -> Result<Vec<u8>, String> {
    // Convert to RGBA
    let rgba = img.to_rgba8();

    // Encode to WebP (lossy with quality 80)
    let encoder = webp::Encoder::from_rgba(&rgba, rgba.width(), rgba.height());
    let webp_data = encoder.encode(80.0);

    Ok(webp_data.to_vec())
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::cache::RedisCache;
    use crate::db;
    use crate::routes;
    use crate::state::AppState;

    async fn setup() -> (axum::Router, sqlx::PgPool) {
        dotenvy::dotenv().ok();
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let pool = db::create_pool(&database_url)
            .await
            .expect("pool should connect");
        let redis = RedisCache::new(&redis_url)
            .await
            .expect("redis should connect");

        let state = AppState {
            db: pool.clone(),
            jwt_secret: "test_secret".to_string(),
            redis,
            job_sender: None,
        };

        (routes::router(state), pool)
    }

    async fn body_json(response: axum::http::Response<axum::body::Body>) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn get_test_token(app: &axum::Router, email: &str) -> String {
        // Register user
        let register_body = serde_json::json!({
            "email": email,
            "password": "password123",
            "display_name": "Test User"
        });

        let register_request = Request::builder()
            .method("POST")
            .uri("/api/auth/register")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::to_string(&register_body).unwrap(),
            ))
            .unwrap();

        let _register_response = app.clone().oneshot(register_request).await.unwrap();

        // Login to get token
        let login_body = serde_json::json!({
            "email": email,
            "password": "password123"
        });

        let login_request = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::to_string(&login_body).unwrap(),
            ))
            .unwrap();

        let login_response = app.clone().oneshot(login_request).await.unwrap();
        let login_json = body_json(login_response).await;
        login_json["token"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn upload_without_auth_returns_401() {
        let (app, _pool) = setup().await;

        // Create a simple test image (1x1 pixel PNG)
        let img = image::DynamicImage::new_rgb8(1, 1);
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        let png_data = buf.into_inner();

        let boundary = "----TestBoundary";
        let body = format!(
            "--{}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"test.png\"\r\n\
             Content-Type: image/png\r\n\
             \r\n",
            boundary
        );
        let mut body = body.into_bytes();
        body.extend_from_slice(&png_data);
        body.extend_from_slice(
            format!("\r\n--{}--\r\n", boundary).as_bytes(),
        );

        let request = Request::builder()
            .method("POST")
            .uri("/api/admin/upload")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(axum::body::Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn upload_invalid_file_type_returns_400() {
        let (app, _pool) = setup().await;
        let token = get_test_token(&app, "test_upload@example.com").await;

        // Create a text file (invalid type)
        let text_data = b"This is not an image";

        let boundary = "----TestBoundary";
        let body = format!(
            "--{}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\n\
             Content-Type: text/plain\r\n\
             \r\n",
            boundary
        );
        let mut body = body.into_bytes();
        body.extend_from_slice(text_data);
        body.extend_from_slice(
            format!("\r\n--{}--\r\n", boundary).as_bytes(),
        );

        let request = Request::builder()
            .method("POST")
            .uri("/api/admin/upload")
            .header("authorization", format!("Bearer {}", token))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(axum::body::Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = body_json(response).await;
        assert!(json["error"]
            .as_str()
            .unwrap()
            .contains("Invalid file type"));
    }

    #[tokio::test]
    async fn upload_file_too_large_returns_error() {
        let (app, _pool) = setup().await;
        let token = get_test_token(&app, "test_large@example.com").await;

        // Create a large file (11MB of zeros - not a valid image, but tests size check)
        // Note: This test checks size validation before image decoding
        let large_data = vec![0u8; 11 * 1024 * 1024];

        let boundary = "----TestBoundary";
        let body = format!(
            "--{}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"large.png\"\r\n\
             Content-Type: image/png\r\n\
             \r\n",
            boundary
        );
        let mut body = body.into_bytes();
        body.extend_from_slice(&large_data);
        body.extend_from_slice(
            format!("\r\n--{}--\r\n", boundary).as_bytes(),
        );

        let request = Request::builder()
            .method("POST")
            .uri("/api/admin/upload")
            .header("authorization", format!("Bearer {}", token))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(axum::body::Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        // Body limit returns 413, our validation returns 400
        assert!(
            status == StatusCode::PAYLOAD_TOO_LARGE || status == StatusCode::BAD_REQUEST,
            "Expected 413 or 400, got {}",
            status
        );
    }

    #[tokio::test]
    async fn upload_valid_png_returns_201() {
        let (app, _pool) = setup().await;
        let token = get_test_token(&app, "test_valid_upload@example.com").await;

        // Create a test PNG image (100x50 pixels)
        let img = image::DynamicImage::new_rgb8(100, 50);
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        let png_data = buf.into_inner();

        let boundary = "----TestBoundary";
        let body = format!(
            "--{}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"test.png\"\r\n\
             Content-Type: image/png\r\n\
             \r\n",
            boundary
        );
        let mut body = body.into_bytes();
        body.extend_from_slice(&png_data);
        body.extend_from_slice(
            format!("\r\n--{}--\r\n", boundary).as_bytes(),
        );

        let request = Request::builder()
            .method("POST")
            .uri("/api/admin/upload")
            .header("authorization", format!("Bearer {}", token))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(axum::body::Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let json = body_json(response).await;
        assert!(json["path"]
            .as_str()
            .unwrap()
            .contains("/uploads/products/"));
        assert!(json["path"]
            .as_str()
            .unwrap()
            .ends_with("/optimized.webp"));
        assert_eq!(json["width"].as_u64().unwrap(), 1080);
        assert_eq!(json["height"].as_u64().unwrap(), 1080);
    }

    #[tokio::test]
    async fn upload_large_image_is_resized() {
        let (app, _pool) = setup().await;
        let token = get_test_token(&app, "test_resize@example.com").await;

        // Create a large test PNG image (2000x1000 pixels)
        let img = image::DynamicImage::new_rgb8(2000, 1000);
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        let png_data = buf.into_inner();

        let boundary = "----TestBoundary";
        let body = format!(
            "--{}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"large.png\"\r\n\
             Content-Type: image/png\r\n\
             \r\n",
            boundary
        );
        let mut body = body.into_bytes();
        body.extend_from_slice(&png_data);
        body.extend_from_slice(
            format!("\r\n--{}--\r\n", boundary).as_bytes(),
        );

        let request = Request::builder()
            .method("POST")
            .uri("/api/admin/upload")
            .header("authorization", format!("Bearer {}", token))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(axum::body::Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let json = body_json(response).await;
        // Should be cropped and scaled to exactly 1080x1080
        assert_eq!(json["width"].as_u64().unwrap(), 1080);
        assert_eq!(json["height"].as_u64().unwrap(), 1080);
    }

    #[tokio::test]
    async fn upload_jpeg_returns_201() {
        let (app, _pool) = setup().await;
        let token = get_test_token(&app, "test_jpeg@example.com").await;

        // Create a test JPEG image
        let img = image::DynamicImage::new_rgb8(80, 60);
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Jpeg)
            .unwrap();
        let jpeg_data = buf.into_inner();

        let boundary = "----TestBoundary";
        let body = format!(
            "--{}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"test.jpg\"\r\n\
             Content-Type: image/jpeg\r\n\
             \r\n",
            boundary
        );
        let mut body = body.into_bytes();
        body.extend_from_slice(&jpeg_data);
        body.extend_from_slice(
            format!("\r\n--{}--\r\n", boundary).as_bytes(),
        );

        let request = Request::builder()
            .method("POST")
            .uri("/api/admin/upload")
            .header("authorization", format!("Bearer {}", token))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(axum::body::Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let json = body_json(response).await;
        assert!(json["path"]
            .as_str()
            .unwrap()
            .ends_with("/optimized.webp"));
    }
}
