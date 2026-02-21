-- Migration 009: Daemon heartbeat tracking table
CREATE TABLE IF NOT EXISTS daemon_status (
    id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    last_heartbeat TIMESTAMPTZ,
    started_at TIMESTAMPTZ,
    pid INTEGER,
    version TEXT,
    worker_states JSONB DEFAULT '{}'::jsonb
);
INSERT INTO daemon_status (id) VALUES (1) ON CONFLICT DO NOTHING;
