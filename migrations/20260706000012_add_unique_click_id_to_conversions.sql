-- ============================================================
-- Migration 12: Add UNIQUE constraint to conversions.click_id
-- Prevents duplicate conversions from the same click event
-- ============================================================

-- First, clean up any existing duplicates (keep earliest by created_at)
DELETE FROM conversions
WHERE id NOT IN (
    SELECT DISTINCT ON (click_id) id
    FROM conversions
    ORDER BY click_id, created_at ASC
);

-- Add unique constraint
ALTER TABLE conversions ADD CONSTRAINT uq_conversions_click_id UNIQUE (click_id);

-- Add index for postback rate limiting
CREATE INDEX IF NOT EXISTS idx_conversions_created_at ON conversions(created_at);
