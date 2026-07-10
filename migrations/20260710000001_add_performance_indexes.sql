-- Performance indexes for common query patterns
-- Created: 2026-07-10

-- Products: category filtering + soft delete + ordering
CREATE INDEX IF NOT EXISTS idx_products_category ON products (category) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_products_created_at ON products (created_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_products_deleted_at ON products (deleted_at);

-- Products: full-text search (Thai + English)
-- Add tsvector column for full-text search
ALTER TABLE products ADD COLUMN IF NOT EXISTS search_vector tsvector;

-- Populate search_vector from title + description
UPDATE products SET search_vector =
    setweight(to_tsvector('simple', COALESCE(title, '')), 'A') ||
    setweight(to_tsvector('simple', COALESCE(description, '')), 'B');

-- GIN index for fast full-text search
CREATE INDEX IF NOT EXISTS idx_products_search ON products USING GIN (search_vector);

-- Auto-update search_vector on INSERT/UPDATE
CREATE OR REPLACE FUNCTION products_search_vector_update() RETURNS trigger AS $$
BEGIN
    NEW.search_vector :=
        setweight(to_tsvector('simple', COALESCE(NEW.title, '')), 'A') ||
        setweight(to_tsvector('simple', COALESCE(NEW.description, '')), 'B');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_products_search_vector ON products;
CREATE TRIGGER trg_products_search_vector
    BEFORE INSERT OR UPDATE OF title, description ON products
    FOR EACH ROW
    EXECUTE FUNCTION products_search_vector_update();

-- Merchant links: FK lookups
CREATE INDEX IF NOT EXISTS idx_merchant_links_product_id ON merchant_links (product_id);

-- Product variants: FK lookups
CREATE INDEX IF NOT EXISTS idx_product_variants_merchant_link_id ON product_variants (merchant_link_id);

-- Conversions: postback lookups by click_id
CREATE INDEX IF NOT EXISTS idx_conversions_click_id ON conversions (click_id);

-- Conversions: admin listing by creator
CREATE INDEX IF NOT EXISTS idx_conversions_creator_id ON conversions (creator_id);
