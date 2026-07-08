use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct ScrapeSource {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub name: String,
    pub platform: String,
    pub source_url: String,
    pub scrape_config: serde_json::Value,
    pub is_active: bool,
    pub scrape_interval_hours: i32,
    pub last_scraped_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
