-- ============================================================
-- Migration 9: Scrape Sources, Jobs, and Results
-- ============================================================

-- Scrape Sources: Where to scrape from
CREATE TABLE IF NOT EXISTS scrape_sources (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    creator_id          UUID NOT NULL REFERENCES creators(id) ON DELETE CASCADE,
    name                VARCHAR(255) NOT NULL,
    platform            VARCHAR(50) NOT NULL,  -- 'shopee', 'lazada', 'tiktok', 'rss', 'custom'
    source_url          TEXT NOT NULL,
    scrape_config       JSONB NOT NULL DEFAULT '{}',  -- platform-specific config
    is_active           BOOLEAN NOT NULL DEFAULT true,
    scrape_interval_hours INT NOT NULL DEFAULT 24,
    last_scraped_at     TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_scrape_sources_creator_id ON scrape_sources(creator_id);
CREATE INDEX IF NOT EXISTS idx_scrape_sources_platform ON scrape_sources(platform);
CREATE INDEX IF NOT EXISTS idx_scrape_sources_active ON scrape_sources(is_active) WHERE is_active = true;

-- Scrape Jobs: Individual scraping tasks
CREATE TABLE IF NOT EXISTS scrape_jobs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_id           UUID NOT NULL REFERENCES scrape_sources(id) ON DELETE CASCADE,
    creator_id          UUID NOT NULL REFERENCES creators(id) ON DELETE CASCADE,
    status              VARCHAR(20) NOT NULL DEFAULT 'pending',  -- 'pending', 'running', 'completed', 'failed'
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    error_message       TEXT,
    items_found         INT NOT NULL DEFAULT 0,
    items_ingested      INT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_scrape_jobs_source_id ON scrape_jobs(source_id);
CREATE INDEX IF NOT EXISTS idx_scrape_jobs_creator_id ON scrape_jobs(creator_id);
CREATE INDEX IF NOT EXISTS idx_scrape_jobs_status ON scrape_jobs(status);

-- Scrape Results: Raw scraped data before ingestion
CREATE TABLE IF NOT EXISTS scrape_results (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id              UUID NOT NULL REFERENCES scrape_jobs(id) ON DELETE CASCADE,
    raw_data            JSONB NOT NULL,
    ingested_at         TIMESTAMPTZ,
    product_id          UUID REFERENCES products(id) ON DELETE SET NULL,
    merchant_link_id    UUID REFERENCES merchant_links(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_scrape_results_job_id ON scrape_results(job_id);

-- Add trigger for scrape_sources updated_at
CREATE TRIGGER update_scrape_sources_updated_at BEFORE UPDATE ON scrape_sources
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
