-- Phase 25: Reasoning Agent
-- salience_audit_log: tracks x1.3 / x0.1 / x0.9 stability boosts per reasoning run
--   for idempotency (revert-by-run_id) and operator audit of REAS-10 side-effects.
--
-- Reviews revision (HIGH #1): UNIQUE (run_id, memory_id) guarantees idempotency —
-- the same (run_id, memory_id) pair can be logged at most ONCE. apply_stability_boost
-- uses ON CONFLICT DO NOTHING and short-circuits the stability multiplication when
-- a row already exists, so retries never double-boost.

CREATE TABLE IF NOT EXISTS salience_audit_log (
    id BIGSERIAL PRIMARY KEY,
    run_id TEXT NOT NULL,
    memory_id UUID NOT NULL,
    magnitude DOUBLE PRECISION NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('final_selection', 'tombstoned', 'discarded', 'create_memory_source')),
    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    prev_stability DOUBLE PRECISION NOT NULL,
    new_stability DOUBLE PRECISION NOT NULL,
    UNIQUE (run_id, memory_id)
);

CREATE INDEX IF NOT EXISTS idx_salience_audit_run_id ON salience_audit_log (run_id);
CREATE INDEX IF NOT EXISTS idx_salience_audit_memory_id ON salience_audit_log (memory_id);
-- Note: UNIQUE (run_id, memory_id) constraint auto-creates a unique btree index,
-- so no separate CREATE UNIQUE INDEX is needed.
