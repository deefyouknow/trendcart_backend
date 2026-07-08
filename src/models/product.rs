use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow, Serialize)]
pub struct Product {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub title: String,
    pub description: String,
    pub category: String,
    pub images: serde_json::Value,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
