use dotenvy::dotenv;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::env;

pub async fn test_pool() -> PgPool {
    dotenv().ok();
    let database_url =
        env::var("DATABASE_URL").expect("DATABASE_URL must be set in .env for tests");
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("failed to connect to test database")
}
