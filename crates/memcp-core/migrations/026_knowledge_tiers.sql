-- Phase 24: Knowledge Tiers
-- knowledge_tier: classifies memories by provenance/quality level
-- source_ids: JSONB array of UUID strings linking derived memories to their evidence

ALTER TABLE memories ADD COLUMN IF NOT EXISTS knowledge_tier TEXT NOT NULL DEFAULT 'explicit'
    CHECK (knowledge_tier IN ('raw', 'imported', 'explicit', 'derived', 'pattern'));
ALTER TABLE memories ADD COLUMN IF NOT EXISTS source_ids JSONB;

-- Index for tier-based WHERE filtering (e.g., exclude raw by default)
CREATE INDEX IF NOT EXISTS idx_memories_knowledge_tier ON memories (knowledge_tier);

-- GIN index for containment queries on source_ids (find derived memories referencing a given source)
CREATE INDEX IF NOT EXISTS idx_memories_source_ids_gin ON memories USING GIN (source_ids)
    WHERE source_ids IS NOT NULL;

-- TIER-03: Backfill existing memories by write_path (per D-02)
UPDATE memories SET knowledge_tier = CASE
    WHEN write_path IN ('auto_store', 'session_summary') THEN 'raw'
    WHEN write_path IN ('explicit_store', 'annotation') THEN 'explicit'
    WHEN write_path = 'import' THEN 'imported'
    ELSE 'explicit'
END;
