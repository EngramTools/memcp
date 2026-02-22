-- Migration 011: Extend daemon_status with sidecar ingest tracking and model info
ALTER TABLE daemon_status
    ADD COLUMN IF NOT EXISTS last_ingest_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS ingest_count_today INTEGER DEFAULT 0,
    ADD COLUMN IF NOT EXISTS ingest_date DATE,
    ADD COLUMN IF NOT EXISTS watched_file_count INTEGER DEFAULT 0,
    ADD COLUMN IF NOT EXISTS embedding_model TEXT,
    ADD COLUMN IF NOT EXISTS embedding_dimension INTEGER;
