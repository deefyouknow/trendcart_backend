-- Convert price from DECIMAL to DOUBLE PRECISION for sqlx f64 compatibility
ALTER TABLE product_variants ALTER COLUMN price TYPE DOUBLE PRECISION USING price::DOUBLE PRECISION;
