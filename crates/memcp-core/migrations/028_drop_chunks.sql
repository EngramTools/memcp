-- Migration 028: Drop chunk columns from memories.
-- Prerequisite: crates/memcp-core/src/bin/migrate_028_collapse_chunks MUST run BEFORE this migration applies.
-- That binary reassembles parents, re-embeds them, and DELETEs chunk rows.
-- This file is DDL-only (Research R-1: vectorization is async network I/O, cannot live inside sqlx::migrate!).
-- Idempotent per CONTEXT.md D-02.

DROP INDEX IF EXISTS idx_memories_parent_id;
ALTER TABLE memories DROP COLUMN IF EXISTS parent_id;
ALTER TABLE memories DROP COLUMN IF EXISTS chunk_index;
ALTER TABLE memories DROP COLUMN IF EXISTS total_chunks;
