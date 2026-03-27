-- Migration 024: Knowledge graph tables for normalized entity/relationship storage
-- Supports Phase 7 Knowledge Graph:
--   - entities: canonical entity nodes with alias support
--   - entity_mentions: links entities to source memories
--   - entity_relationships: typed, temporal relationships between entities
--   - entity_facts: attribute/value facts about entities with temporal validity
--   - entity_normalization_status on memories: tracks migration from flat JSONB

-- Enable btree_gist for potential future exclusion constraints on temporal ranges
CREATE EXTENSION IF NOT EXISTS btree_gist;

-- Canonical entity nodes
CREATE TABLE IF NOT EXISTS entities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    aliases JSONB NOT NULL DEFAULT '[]',
    metadata JSONB NOT NULL DEFAULT '{}',
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (name, entity_type)
);

-- Links between entities and the memories that mention them
CREATE TABLE IF NOT EXISTS entity_mentions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_id UUID NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    context_snippet TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Typed, temporal relationships between entity pairs
CREATE TABLE IF NOT EXISTS entity_relationships (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_id UUID NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    object_id UUID NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    predicate TEXT NOT NULL,
    relationship_type TEXT NOT NULL CHECK (relationship_type IN ('temporal', 'entity', 'semantic', 'causal')),
    weight FLOAT NOT NULL DEFAULT 1.0,
    valid_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    invalid_at TIMESTAMPTZ,
    superseded_by UUID REFERENCES entity_relationships(id),
    source_memory_id TEXT REFERENCES memories(id) ON DELETE SET NULL,
    confidence FLOAT NOT NULL DEFAULT 1.0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Attribute/value facts about entities, with temporal validity
CREATE TABLE IF NOT EXISTS entity_facts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_id UUID NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    attribute TEXT NOT NULL,
    value JSONB NOT NULL,
    valid_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    invalid_at TIMESTAMPTZ,
    source_memory_id TEXT REFERENCES memories(id) ON DELETE SET NULL,
    confidence FLOAT NOT NULL DEFAULT 1.0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Track normalization progress for the dual-read migration window
ALTER TABLE memories ADD COLUMN IF NOT EXISTS entity_normalization_status TEXT NOT NULL DEFAULT 'pending';

-- Indexes for entities
CREATE INDEX IF NOT EXISTS idx_entities_aliases
    ON entities USING GIN (aliases);

CREATE INDEX IF NOT EXISTS idx_entities_entity_type
    ON entities(entity_type);

-- Indexes for entity_mentions
CREATE INDEX IF NOT EXISTS idx_entity_mentions_memory_id
    ON entity_mentions(memory_id);

CREATE INDEX IF NOT EXISTS idx_entity_mentions_entity_id
    ON entity_mentions(entity_id);

-- Indexes for entity_relationships
-- Partial index on active (non-invalidated) relationships — critical for traversal
CREATE INDEX IF NOT EXISTS idx_entity_relationships_active
    ON entity_relationships(subject_id, predicate)
    WHERE invalid_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_entity_relationships_subject_predicate
    ON entity_relationships(subject_id, predicate);

CREATE INDEX IF NOT EXISTS idx_entity_relationships_object
    ON entity_relationships(object_id)
    WHERE invalid_at IS NULL;

-- Indexes for entity_facts
-- Partial index on active (non-invalidated) facts — critical for fact lookups
CREATE INDEX IF NOT EXISTS idx_entity_facts_active
    ON entity_facts(entity_id, attribute)
    WHERE invalid_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_entity_facts_entity_attribute
    ON entity_facts(entity_id, attribute);

-- Index for normalization backfill queries
CREATE INDEX IF NOT EXISTS idx_memories_normalization_status
    ON memories(entity_normalization_status)
    WHERE entity_normalization_status = 'pending';
