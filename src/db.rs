use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use std::time::Duration;

pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let options: PgConnectOptions = database_url.parse()?;

    PgPoolOptions::new()
        .min_connections(2)
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(30))
        .max_lifetime(Duration::from_secs(600))
        .idle_timeout(Duration::from_secs(60))
        .test_before_acquire(true)
        .connect_with(options)
        .await
}

/// Execute a query with one retry on transient connection errors.
/// Handles "Operation timed out" and "Broken pipe" from unstable remote DBs.
pub async fn query_with_retry<F, Fut, T>(f: F) -> Result<T, sqlx::Error>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, sqlx::Error>>,
{
    match f().await {
        Ok(val) => Ok(val),
        Err(sqlx::Error::Io(ref io_err))
            if io_err.kind() == std::io::ErrorKind::TimedOut
                || io_err.raw_os_error() == Some(32) =>
        {
            tracing::warn!("transient DB error, retrying once: {}", io_err);
            f().await
        }
        Err(sqlx::Error::Database(ref db_err))
            if db_err.message().contains("closed") || db_err.message().contains("terminated") =>
        {
            tracing::warn!("connection terminated, retrying once: {}", db_err.message());
            f().await
        }
        Err(sqlx::Error::PoolTimedOut) => {
            tracing::warn!("pool timed out, retrying once");
            f().await
        }
        Err(e) => Err(e),
    }
}
