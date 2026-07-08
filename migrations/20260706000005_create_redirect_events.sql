CREATE TABLE IF NOT EXISTS redirect_events (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    merchant_link_id  UUID NOT NULL REFERENCES merchant_links(id) ON DELETE CASCADE,
    variant_id        UUID REFERENCES product_variants(id) ON DELETE SET NULL,
    click_id          UUID NOT NULL DEFAULT gen_random_uuid(),
    ip_address        INET,
    user_agent        TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_redirect_events_merchant_link_id ON redirect_events(merchant_link_id);
CREATE INDEX IF NOT EXISTS idx_redirect_events_created_at ON redirect_events(created_at);
CREATE INDEX IF NOT EXISTS idx_redirect_events_click_id ON redirect_events(click_id);

-- ============================================================
-- Helper: updated_at trigger (auto-update on row change)
-- ============================================================
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

CREATE TRIGGER update_creators_updated_at BEFORE UPDATE ON creators
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_products_updated_at BEFORE UPDATE ON products
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_merchant_links_updated_at BEFORE UPDATE ON merchant_links
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_product_variants_updated_at BEFORE UPDATE ON product_variants
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
