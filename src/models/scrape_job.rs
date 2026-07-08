use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct ScrapeJob {
    pub id: Uuid,
    pub source_id: Uuid,
    pub creator_id: Uuid,
    pub status: String,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub items_found: i32,
    pub items_ingested: i32,
    pub created_at: DateTime<Utc>,
}
