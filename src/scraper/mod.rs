pub mod registry;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A single scraped product with all its merchant links and variants
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapedProduct {
    pub title: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub images: Vec<String>,
    pub merchant_links: Vec<ScrapedMerchantLink>,
}

/// A merchant link scraped from a platform
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapedMerchantLink {
    pub platform: String,
    pub store_name: String,
    pub affiliate_url: String,
    pub variants: Vec<ScrapedVariant>,
}

/// A product variant with price and stock info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapedVariant {
    pub variant_name: String,
    pub price: f64,
    pub currency: String,
    pub stock_status: String,
}

/// Errors that can occur during scraping
#[derive(Debug, thiserror::Error)]
pub enum ScraperError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Not implemented for platform: {0}")]
    NotImplemented(String),
}

/// Trait that platform scrapers must implement
#[async_trait]
pub trait Scraper: Send + Sync {
    /// Scrape products from the given source URL
    async fn scrape(
        &self,
        source_url: &str,
        config: &serde_json::Value,
    ) -> Result<Vec<ScrapedProduct>, ScraperError>;

    /// Return the platform name this scraper handles
    fn platform(&self) -> &str;
}
