-- Migration 019: Add session_tags column for topic accumulation
-- Session tags are a denormalized JSONB array of tag strings accumulated from recalled memories.
-- Used for implicit tag-affinity boosting on subsequent recalls within the same session.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS session_tags JSONB;
