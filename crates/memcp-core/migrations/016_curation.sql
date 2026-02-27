-- Curation run tracking and action history for AI Brain Curation.
-- Enables periodic memory self-maintenance with full undo capability.

CREATE TABLE curation_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'running',
    mode TEXT NOT NULL DEFAULT 'auto',
    window_start TIMESTAMPTZ,
    window_end TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    merged_count INT NOT NULL DEFAULT 0,
    flagged_stale_count INT NOT NULL DEFAULT 0,
    strengthened_count INT NOT NULL DEFAULT 0,
    skipped_count INT NOT NULL DEFAULT 0,
    error_message TEXT
);

CREATE TABLE curation_actions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES curation_runs(id) ON DELETE CASCADE,
    action_type TEXT NOT NULL,
    target_memory_ids TEXT[] NOT NULL,
    merged_memory_id TEXT,
    original_salience FLOAT8,
    details JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_curation_actions_run_id ON curation_actions(run_id);
CREATE INDEX idx_curation_runs_status ON curation_runs(status);
CREATE INDEX idx_curation_runs_completed_at ON curation_runs(completed_at DESC);
