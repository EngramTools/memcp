-- Add chunking support to memories table.
-- Chunks are stored as regular memory rows with parent_id linking to the original.
-- ON DELETE CASCADE ensures chunks are cleaned up when parent is deleted.
ALTER TABLE memories ADD COLUMN parent_id TEXT REFERENCES memories(id) ON DELETE CASCADE;
ALTER TABLE memories ADD COLUMN chunk_index INTEGER;
ALTER TABLE memories ADD COLUMN total_chunks INTEGER;

-- Index for fast chunk lookups by parent
CREATE INDEX idx_memories_parent_id ON memories(parent_id);
