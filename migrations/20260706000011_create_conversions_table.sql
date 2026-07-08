-- ============================================================
-- Migration 11: conversions (Phase 9 — Conversion Tracking)
-- ============================================================

CREATE TABLE IF NOT EXISTS conversions (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    click_id          UUID NOT NULL REFERENCES redirect_events(click_id) ON DELETE CASCADE,
    merchant_link_id  UUID NOT NULL REFERENCES merchant_links(id) ON DELETE CASCADE,
    creator_id        UUID NOT NULL REFERENCES creators(id) ON DELETE CASCADE,
    platform          VARCHAR(50) NOT NULL,              -- 'shopee', 'lazada', 'tiktok', etc.
    order_id          VARCHAR(255),                      -- platform's order ID
    order_amount      DOUBLE PRECISION,                  -- order total
    currency          VARCHAR(3) NOT NULL DEFAULT 'THB',
    commission        DOUBLE PRECISION,                  -- affiliate commission earned
    status            VARCHAR(20) NOT NULL DEFAULT 'pending', -- 'pending', 'approved', 'rejected', 'cancelled'
    conversion_type   VARCHAR(50) NOT NULL DEFAULT 'sale',    -- 'sale', 'lead', 'install'
    product_name      VARCHAR(500),                      -- product name from platform
    raw_data          JSONB NOT NULL DEFAULT '{}',       -- raw postback payload
    postback_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(), -- when platform sent the postback
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_conversions_click_id ON conversions(click_id);
CREATE INDEX IF NOT EXISTS idx_conversions_creator_id ON conversions(creator_id);
CREATE INDEX IF NOT EXISTS idx_conversions_merchant_link_id ON conversions(merchant_link_id);
CREATE INDEX IF NOT EXISTS idx_conversions_status ON conversions(status);
CREATE INDEX IF NOT EXISTS idx_conversions_postback_at ON conversions(postback_at);
CREATE INDEX IF NOT EXISTS idx_conversions_order_id ON conversions(order_id);

CREATE TRIGGER update_conversions_updated_at BEFORE UPDATE ON conversions
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
