DO $$ BEGIN
    CREATE TYPE stock_status_enum AS ENUM ('in_stock', 'out_of_stock', 'unknown');
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

CREATE TABLE IF NOT EXISTS product_variants (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    merchant_link_id  UUID NOT NULL REFERENCES merchant_links(id) ON DELETE CASCADE,
    variant_name      VARCHAR(255) NOT NULL,
    price             DECIMAL(10, 2) NOT NULL,
    currency          VARCHAR(3) NOT NULL DEFAULT 'THB',
    stock_status      stock_status_enum NOT NULL DEFAULT 'unknown'
);

CREATE INDEX IF NOT EXISTS idx_product_variants_merchant_link_id ON product_variants(merchant_link_id);
