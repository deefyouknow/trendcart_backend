DO $$ BEGIN
    CREATE TYPE platform_enum AS ENUM ('shopee', 'lazada', 'tiktok', 'other');
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

CREATE TABLE IF NOT EXISTS merchant_links (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    product_id          UUID NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    platform            platform_enum NOT NULL,
    store_name          VARCHAR(255) NOT NULL,
    affiliate_url       TEXT NOT NULL,
    is_price_estimated  BOOLEAN NOT NULL DEFAULT TRUE,
    price_checked_at    TIMESTAMPTZ,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_merchant_links_product_id ON merchant_links(product_id);
CREATE INDEX IF NOT EXISTS idx_merchant_links_platform ON merchant_links(platform);
