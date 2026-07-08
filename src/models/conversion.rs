use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct Conversion {
    pub id: Uuid,
    pub click_id: Uuid,
    pub merchant_link_id: Uuid,
    pub creator_id: Uuid,
    pub platform: String,
    pub order_id: Option<String>,
    pub order_amount: Option<f64>,
    pub currency: String,
    pub commission: Option<f64>,
    pub status: String,
    pub conversion_type: String,
    pub product_name: Option<String>,
    pub raw_data: serde_json::Value,
    pub postback_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
