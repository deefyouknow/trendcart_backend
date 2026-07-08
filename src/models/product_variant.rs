use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow, Serialize)]
pub struct ProductVariant {
    pub id: Uuid,
    pub merchant_link_id: Uuid,
    pub variant_name: String,
    pub price: f64,
    pub currency: String,
    pub stock_status: String,
}
