-- Add embedding tier tracking for multi-model support.
-- Each embedding now belongs to a tier (e.g., 'fast' for local model, 'quality' for API model).
-- The DEFAULT 'fast' ensures all existing rows are backfilled automatically by Postgres.

ALTER TABLE memory_embeddings ADD COLUMN IF NOT EXISTS tier TEXT NOT NULL DEFAULT 'fast';

-- Index for tier-based queries (promotion sweep, lazy quality check)
CREATE INDEX IF NOT EXISTS idx_memory_embeddings_tier ON memory_embeddings (tier) WHERE is_current = true;

-- Composite index for promotion candidate queries
CREATE INDEX IF NOT EXISTS idx_memory_embeddings_tier_memory ON memory_embeddings (tier, memory_id) WHERE is_current = true;
