use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow, Serialize)]
pub struct MerchantLink {
    pub id: Uuid,
    pub product_id: Uuid,
    pub platform: String,
    pub store_name: String,
    pub affiliate_url: String,
    pub is_price_estimated: bool,
    pub price_checked_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}
