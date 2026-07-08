mod common;

use backend::db;

#[tokio::test]
async fn test_create_pool_connects_successfully() {
    let pool = common::test_pool().await;
    let row: (i32,) = sqlx::query_as("SELECT 1")
        .fetch_one(&pool)
        .await
        .expect("basic query should succeed against the pool");
    assert_eq!(row.0, 1);

    // Also directly exercise db::create_pool itself.
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set in .env for tests");
    let direct_pool = db::create_pool(&database_url)
        .await
        .expect("create_pool should succeed");
    let direct_row: (i32,) = sqlx::query_as("SELECT 1")
        .fetch_one(&direct_pool)
        .await
        .expect("basic query via create_pool should succeed");
    assert_eq!(direct_row.0, 1);
}
