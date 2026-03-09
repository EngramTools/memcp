-- Provenance: trust_level, session_id, agent_role, metadata columns
-- Enables recording WHO wrote WHAT with WHAT authority on every memory (OWASP ASI06).

ALTER TABLE memories ADD COLUMN IF NOT EXISTS trust_level REAL NOT NULL DEFAULT 0.5;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS session_id TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS agent_role TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS metadata JSONB NOT NULL DEFAULT '{}'::jsonb;

CREATE INDEX IF NOT EXISTS idx_memories_trust_level ON memories(trust_level);
CREATE INDEX IF NOT EXISTS idx_memories_session_id ON memories(session_id) WHERE session_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_memories_agent_role ON memories(agent_role) WHERE agent_role IS NOT NULL;
