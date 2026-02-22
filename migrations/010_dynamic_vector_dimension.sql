-- Remove fixed 384-dimension constraint from embedding column.
-- Column becomes untyped vector -- any dimension accepted.
-- HNSW index dropped; daemon recreates it at startup with configured dimension.
--
-- Background: migration 002 created embedding as vector(384) which hardcodes the
-- fastembed All-MiniLM-L6-v2 dimension. Phase 07 (Modularity) makes the embedding
-- model configurable, so different models may have different dimensions.
-- Migration 003 created the HNSW index for vector(384); we drop it here and let
-- the daemon recreate it with the correct dimension-aware cast at startup.
--
-- Existing 384-dim embeddings remain valid -- untyped vector accepts any dimension.

DROP INDEX IF EXISTS idx_memory_embeddings_hnsw;

ALTER TABLE memory_embeddings
    ALTER COLUMN embedding TYPE vector
    USING embedding::vector;
