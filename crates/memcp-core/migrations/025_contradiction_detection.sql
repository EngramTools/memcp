-- Migration 025: Contradiction detection support
-- Adds predicate_contradictions lookup table used by detect_relationship_contradictions()
-- to identify semantically opposing predicates without embedding/semantic analysis.

CREATE TABLE IF NOT EXISTS predicate_contradictions (
    predicate_a TEXT NOT NULL,
    predicate_b TEXT NOT NULL,
    PRIMARY KEY (predicate_a, predicate_b)
);

-- Seed common contradiction pairs (bidirectional)
INSERT INTO predicate_contradictions (predicate_a, predicate_b) VALUES
    ('employed_by', 'not_employed_by'),
    ('not_employed_by', 'employed_by'),
    ('active', 'inactive'),
    ('inactive', 'active'),
    ('approved', 'rejected'),
    ('rejected', 'approved'),
    ('likes', 'dislikes'),
    ('dislikes', 'likes'),
    ('supports', 'opposes'),
    ('opposes', 'supports')
ON CONFLICT DO NOTHING;
