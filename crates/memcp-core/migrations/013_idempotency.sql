-- Migration 013: Idempotency infrastructure for store operations
--
-- Adds:
--   1. content_hash column on memories for content-hash dedup within a time window
--   2. idempotency_keys table for caller-provided at-most-once keys
--
-- No backfill for existing rows — dedup only covers post-migration stores per design.

ALTER TABLE memories ADD COLUMN IF NOT EXISTS content_hash TEXT;
CREATE INDEX IF NOT EXISTS idx_memories_content_hash ON memories(content_hash, created_at) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS idempotency_keys (
    key TEXT PRIMARY KEY,
    memory_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_idempotency_keys_expires_at ON idempotency_keys(expires_at);
