use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct ScrapeResult {
    pub id: Uuid,
    pub job_id: Uuid,
    pub raw_data: serde_json::Value,
    pub ingested_at: Option<DateTime<Utc>>,
    pub product_id: Option<Uuid>,
    pub merchant_link_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}
