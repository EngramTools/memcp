-- Migration 027: Phase 24.5 — reply_to_id column for conversation threading
--
-- Adds a nullable UUID (stored as TEXT to match existing id column) column to memories.
-- Distinct semantics from parent_id (which is used for chunk->parent chunking from Phase 08):
--   reply_to_id = "this memory logically follows / replies to that memory" (conversation turns)
--   parent_id   = "this memory is a chunk of that memory" (chunking)
-- Per D-16 (no FK, partial index; orphaned reply_to becomes informational).

ALTER TABLE memories ADD COLUMN IF NOT EXISTS reply_to_id TEXT;

CREATE INDEX IF NOT EXISTS idx_memories_reply_to_id
    ON memories (reply_to_id)
    WHERE reply_to_id IS NOT NULL;
