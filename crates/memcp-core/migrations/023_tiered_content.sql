-- Migration 023: Tiered content columns for L0/L1/L2 memory representation
-- Adds abstract_text (L0 ~100 tokens), overview_text (L1 ~500 tokens), and abstraction_status
-- abstraction_status: 'pending' | 'complete' | 'failed' | 'skipped'
-- Short memories (< 200 chars) are marked 'skipped' at store time — abstraction adds no value.

ALTER TABLE memories ADD COLUMN IF NOT EXISTS abstract_text TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS overview_text TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS abstraction_status TEXT NOT NULL DEFAULT 'pending';

CREATE INDEX IF NOT EXISTS idx_memories_abstraction_status ON memories (abstraction_status) WHERE abstraction_status = 'pending';
