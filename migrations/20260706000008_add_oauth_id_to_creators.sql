-- Add oauth_id column to store OAuth provider's user ID (Google sub, GitHub ID)
ALTER TABLE creators ADD COLUMN IF NOT EXISTS oauth_id VARCHAR(255);

-- Create index for faster OAuth lookups
CREATE INDEX IF NOT EXISTS idx_creators_oauth ON creators(oauth_provider, oauth_id);
