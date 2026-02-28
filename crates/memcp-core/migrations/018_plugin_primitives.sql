-- Migration 018: Plugin support primitives
-- Adds event_time, event_time_precision, and workspace columns to memories table.
-- All columns are nullable (NULL defaults) -- fully backward compatible.

ALTER TABLE memories ADD COLUMN IF NOT EXISTS event_time TIMESTAMPTZ;

ALTER TABLE memories ADD COLUMN IF NOT EXISTS event_time_precision TEXT
    CHECK (event_time_precision IN ('decade', 'year', 'month', 'day'));

ALTER TABLE memories ADD COLUMN IF NOT EXISTS workspace TEXT;

-- Partial index on workspace: excludes NULLs for efficiency (NULL = global scope)
CREATE INDEX IF NOT EXISTS memories_workspace_idx ON memories (workspace)
    WHERE workspace IS NOT NULL;
