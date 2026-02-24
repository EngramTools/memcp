-- Migration 012: Memory hygiene — soft-delete, TTL, dedup tracking, GC metrics
ALTER TABLE memories
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS dedup_sources JSONB DEFAULT '[]'::jsonb;

CREATE INDEX IF NOT EXISTS idx_memories_deleted_at ON memories(deleted_at) WHERE deleted_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_memories_expires_at ON memories(expires_at) WHERE expires_at IS NOT NULL;

ALTER TABLE daemon_status
    ADD COLUMN IF NOT EXISTS last_gc_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS gc_pruned_total INTEGER DEFAULT 0,
    ADD COLUMN IF NOT EXISTS gc_dedup_merges INTEGER DEFAULT 0,
    ADD COLUMN IF NOT EXISTS filter_stats JSONB DEFAULT '{}'::jsonb;
