use sqlx::PgPool;
use tokio::sync::mpsc;

use crate::cache::RedisCache;
use crate::worker::JobCommand;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt_secret: String,
    pub redis: RedisCache,
    pub job_sender: Option<mpsc::Sender<JobCommand>>,
}

impl AppState {
    /// Invalidate all store-related caches (list + individual product).
    /// Call this on every write to products/variants/merchant_links.
    pub async fn invalidate_store_cache(&self, product_id: Option<uuid::Uuid>) {
        self.redis.delete_pattern("store:list:*").await;
        if let Some(id) = product_id {
            self.redis
                .delete(&format!("store:product:{}", id))
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn app_state_holds_db_pool_and_jwt_secret() {
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
            db: pool,
            jwt_secret: "test_secret".to_string(),
            redis,
            job_sender: None,
        };
        assert_eq!(state.jwt_secret, "test_secret");
    }
}
