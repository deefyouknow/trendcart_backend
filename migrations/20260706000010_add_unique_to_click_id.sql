-- ============================================================
-- Migration 10: Add UNIQUE constraint to redirect_events.click_id
-- Needed for conversions table foreign key reference
-- ============================================================

ALTER TABLE redirect_events ADD CONSTRAINT uq_redirect_events_click_id UNIQUE (click_id);
