-- Convert enum columns to VARCHAR for simpler sqlx handling

-- Create a new type for the conversion
DO $$ BEGIN
    CREATE TYPE platform_enum_new AS ENUM ('shopee', 'lazada', 'tiktok', 'other');
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

-- Alter merchant_links.platform from enum to varchar
ALTER TABLE merchant_links ALTER COLUMN platform TYPE VARCHAR(50);

-- Alter product_variants.stock_status from enum to varchar
ALTER TABLE product_variants ALTER COLUMN stock_status TYPE VARCHAR(50);

-- Drop old enum types
DROP TYPE IF EXISTS platform_enum CASCADE;
DROP TYPE IF EXISTS stock_status_enum CASCADE;
