use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow, Serialize)]
pub struct Creator {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: Option<String>,
    pub oauth_provider: Option<String>,
    pub oauth_id: Option<String>,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
