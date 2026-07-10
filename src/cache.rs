use fred::prelude::*;
use std::time::Duration;

use crate::error::AppError;

/// Redis-backed cache and buffer layer.
///
/// Replaces in-memory StoreCache and RateLimiter.
/// All methods log errors and never panic — callers get `Err` on failure.
#[derive(Clone)]
pub struct RedisCache {
    client: Client,
}

impl RedisCache {
    /// Connect to Redis. Fails fast if unreachable.
    pub async fn new(url: &str) -> Result<Self, String> {
        let config = Config::from_url(url).map_err(|e| format!("invalid REDIS_URL: {}", e))?;
        let client = Builder::from_config(config)
            .with_connection_config(|c| {
                c.connection_timeout = Duration::from_secs(5);
            })
            .build()
            .map_err(|e| format!("fred build error: {}", e))?;

        client
            .init()
            .await
            .map_err(|e| format!("Redis connect failed: {}", e))?;

        tracing::info!("Redis connected");
        Ok(Self { client })
    }

    // ── Liveness ──────────────────────────────────────────────

    /// Ping Redis. Returns Ok(true) if PONG received, Err otherwise.
    pub async fn ping(&self) -> Result<bool, String> {
        self.client
            .ping::<String>(None)
            .await
            .map(|_| true)
            .map_err(|e| format!("Redis PING failed: {}", e))
    }

    // ── Cache-aside (read/write) ──────────────────────────────

    /// GET a cached value. Returns None on miss or error.
    pub async fn get(&self, key: &str) -> Option<String> {
        match self.client.get::<Option<String>, _>(key).await {
            Ok(val) => val,
            Err(e) => {
                tracing::error!("redis GET {} failed: {}", key, e);
                None
            }
        }
    }

    /// SET a value with TTL in seconds.
    pub async fn set(&self, key: &str, value: &str, ttl_secs: u64) {
        let _ = self
            .client
            .set::<(), _, _>(
                key,
                value,
                Some(Expiration::EX(ttl_secs as i64)),
                None,
                false,
            )
            .await
            .map_err(|e| tracing::error!("redis SET {} failed: {}", key, e));
    }

    /// DEL a single key.
    pub async fn delete(&self, key: &str) {
        let _ = self
            .client
            .del::<(), _>(key)
            .await
            .map_err(|e| tracing::error!("redis DEL {} failed: {}", key, e));
    }

    /// Delete all keys matching a glob pattern (SCAN + DEL).
    pub async fn delete_pattern(&self, pattern: &str) {
        let mut cursor: String = "0".to_string();
        loop {
            let result: (String, Vec<String>) = match self
                .client
                .scan_page::<(String, Vec<String>), _, _>(&cursor, pattern, Some(100), None)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("redis SCAN {} failed: {}", pattern, e);
                    return;
                }
            };

            cursor = result.0.clone();
            for key in &result.1 {
                let _ = self.client.del::<(), _>(key).await;
            }

            if cursor == "0" {
                break;
            }
        }
    }

    // ── Session store (refresh tokens) ────────────────────────

    /// Store a refresh token session in Redis.
    /// Key: `session:refresh:{user_id}`  Value: refresh token string.
    pub async fn set_session(&self, user_id: &str, refresh_token: &str, ttl_secs: u64) {
        let key = format!("session:refresh:{}", user_id);
        self.set(&key, refresh_token, ttl_secs).await;
    }

    /// Get the stored refresh token for a user. Returns None if no session or expired.
    pub async fn get_session(&self, user_id: &str) -> Option<String> {
        let key = format!("session:refresh:{}", user_id);
        self.get(&key).await
    }

    /// Delete a user's session (logout / revoke).
    pub async fn delete_session(&self, user_id: &str) {
        let key = format!("session:refresh:{}", user_id);
        self.delete(&key).await;
    }

    // ── Rate limiter (INCR + EXPIRE) ─────────────────────────

    /// Increment a counter key. Sets TTL (seconds) on first increment.
    /// Returns the new count.
    pub async fn increment(&self, key: &str, window_secs: u64) -> Result<u64, AppError> {
        let count: i64 = self
            .client
            .incr::<i64, _>(key)
            .await
            .map_err(|e| {
                tracing::error!("redis INCR {} failed: {}", key, e);
                AppError::Internal("Rate limit check failed".to_string())
            })?;

        // Set TTL only on the first increment (when count == 1)
        if count == 1 {
            let _ = self
                .client
                .expire::<(), _>(key, window_secs as i64, None)
                .await;
        }

        Ok(count as u64)
    }

    // ── Write-behind buffer (LPUSH + RPOP) ────────────────────

    /// Push an event to a list (LPUSH). Used for click/conversion buffering.
    pub async fn push_event(&self, key: &str, value: &str) {
        let _ = self
            .client
            .lpush::<(), _, _>(key, vec![value])
            .await
            .map_err(|e| tracing::error!("redis LPUSH {} failed: {}", key, e));
    }

    /// Pop up to `count` events from a list (RPOP). Used by worker for batch flush.
    pub async fn pop_events(&self, key: &str, count: usize) -> Vec<String> {
        match self.client.rpop::<Vec<String>, _>(key, Some(count)).await {
            Ok(val) => val,
            Err(e) => {
                tracing::error!("redis RPOP {} failed: {}", key, e);
                Vec::new()
            }
        }
    }
}
