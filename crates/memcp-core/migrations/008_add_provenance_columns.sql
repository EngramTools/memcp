-- Migration 008: Add provenance columns for multi-actor tracking
-- Groundwork for Phase 12 (Auth) — values are self-reported until identity is wired.

ALTER TABLE memories ADD COLUMN IF NOT EXISTS actor TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS actor_type TEXT NOT NULL DEFAULT 'agent';
ALTER TABLE memories ADD COLUMN IF NOT EXISTS audience TEXT NOT NULL DEFAULT 'global';

-- Index for filtering by actor (will be heavily used once auth is wired)
CREATE INDEX IF NOT EXISTS idx_memories_actor ON memories(actor) WHERE actor IS NOT NULL;
-- Index for filtering by audience scope
CREATE INDEX IF NOT EXISTS idx_memories_audience ON memories(audience);
