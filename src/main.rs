use std::net::SocketAddr;
use std::time::Duration;

use axum::http::{HeaderValue, Method};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use backend::{cache, db, routes, scraper::registry::ScraperRegistry, state::AppState, worker};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    let frontend_origin = std::env::var("FRONTEND_ORIGIN").expect("FRONTEND_ORIGIN must be set");
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let pool = db::create_pool(&database_url)
        .await
        .expect("failed to create database pool");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("failed to run database migrations");

    let redis = cache::RedisCache::new(&redis_url)
        .await
        .expect("failed to connect to Redis");

    // Initialize scraper registry
    let registry = ScraperRegistry::new();
    // TODO: Register platform scrapers here when available
    // registry.register(Arc::new(ShopeeScraper::new()));

    // Start job worker
    let (mut job_worker, job_sender) = worker::JobWorker::new(pool.clone(), registry);
    tokio::spawn(async move {
        job_worker.run().await;
    });

    let state = AppState {
        db: pool,
        jwt_secret,
        redis,
        job_sender: Some(job_sender),
    };

    let cors = CorsLayer::new()
        .allow_origin(
            frontend_origin
                .parse::<HeaderValue>()
                .expect("FRONTEND_ORIGIN must be a valid header value"),
        )
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::Any)
        .max_age(Duration::from_secs(3600));

    // Serve static files from uploads directory
    let serve_uploads = ServeDir::new("uploads").append_index_html_on_directories(false);

    let app = routes::router(state)
        .layer(cors)
        .nest_service("/uploads", serve_uploads);

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "59123".to_string())
        .parse()
        .expect("PORT must be a valid port number");

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("TrendCart backend listening on {}", addr);

    // Fail hard if port is in use — no auto-pick
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("FATAL: Port {} is already in use: {}", port, e);
            std::process::exit(1);
        });
    axum::serve(listener, app).await.expect("server error");
}
