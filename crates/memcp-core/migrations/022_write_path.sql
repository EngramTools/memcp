-- Write path provenance: how each memory was created.
-- Values: 'session_summary', 'explicit_store', 'annotation', 'import', etc.
-- Enables trust assignment by creation method in Phase 22 agent role guardrails.

ALTER TABLE memories ADD COLUMN IF NOT EXISTS write_path TEXT;
CREATE INDEX IF NOT EXISTS idx_memories_write_path ON memories (write_path) WHERE write_path IS NOT NULL;
