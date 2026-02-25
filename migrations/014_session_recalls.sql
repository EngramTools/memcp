-- Migration 014: Session recall dedup tables
--
-- Adds:
--   1. sessions table — tracks session lifecycle with last_active_at for idle expiry
--   2. session_recalls table — join table for per-session memory dedup (composite PK)
--
-- session_recalls composite PK on (session_id, memory_id) enables ON CONFLICT DO NOTHING
-- for idempotent dedup inserts. FK ON DELETE CASCADE on both FKs ensures:
--   - Deleting a session cascades to all its recall records
--   - Deleting a memory cascades to all recall records referencing it

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_active_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS session_recalls (
    session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    recalled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    relevance REAL NOT NULL,
    PRIMARY KEY (session_id, memory_id)
);

CREATE INDEX IF NOT EXISTS idx_session_recalls_session_id ON session_recalls(session_id);
CREATE INDEX IF NOT EXISTS idx_sessions_last_active ON sessions(last_active_at);
