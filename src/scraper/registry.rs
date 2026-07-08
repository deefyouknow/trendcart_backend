use std::collections::HashMap;
use std::sync::Arc;

use super::Scraper;

/// Registry of scrapers keyed by platform name
pub struct ScraperRegistry {
    scrapers: HashMap<String, Arc<dyn Scraper>>,
}

impl ScraperRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            scrapers: HashMap::new(),
        }
    }

    /// Register a scraper for a platform
    pub fn register(&mut self, scraper: Arc<dyn Scraper>) {
        let platform = scraper.platform().to_string();
        tracing::info!("Registered scraper for platform: {}", platform);
        self.scrapers.insert(platform, scraper);
    }

    /// Get a scraper by platform name
    pub fn get(&self, platform: &str) -> Option<Arc<dyn Scraper>> {
        self.scrapers.get(platform).cloned()
    }

    /// Check if a platform has a registered scraper
    pub fn has_scraper(&self, platform: &str) -> bool {
        self.scrapers.contains_key(platform)
    }

    /// List all registered platforms
    pub fn platforms(&self) -> Vec<&str> {
        self.scrapers.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for ScraperRegistry {
    fn default() -> Self {
        Self::new()
    }
}
