CREATE TABLE IF NOT EXISTS products (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    creator_id  UUID NOT NULL REFERENCES creators(id) ON DELETE CASCADE,
    title       VARCHAR(500) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    category    VARCHAR(100) NOT NULL DEFAULT 'uncategorized',
    images      JSONB NOT NULL DEFAULT '[]',
    deleted_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_products_creator_id ON products(creator_id);
CREATE INDEX IF NOT EXISTS idx_products_category ON products(category);
CREATE INDEX IF NOT EXISTS idx_products_search ON products USING gin(
    to_tsvector('english', coalesce(title, '') || ' ' || coalesce(description, ''))
);
CREATE INDEX IF NOT EXISTS idx_products_not_deleted ON products(deleted_at) WHERE deleted_at IS NULL;
