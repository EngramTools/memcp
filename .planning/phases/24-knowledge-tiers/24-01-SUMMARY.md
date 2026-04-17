---
phase: 24-knowledge-tiers
plan: "01"
subsystem: storage
tags: [rust, migration, knowledge-tier, source-ids, tier-inference, config]

requires:
  - phase: 24-knowledge-tiers
    plan: "00"
    provides: "Test stubs for TIER-01 through TIER-06, MemoryBuilder extensions"
provides:
  - "Migration 026 with knowledge_tier column (TEXT CHECK), source_ids (JSONB), indexes, backfill"
  - "Tier inference from write_path in store() with caller override"
  - "Derived-tier validation requiring non-empty source_ids"
  - "TierWeightsConfig (w_rrf=0.4, w_sal=0.4, w_tier=0.2) in SearchConfig"
  - "tier_score_for() helper mapping tier names to 0.0-1.0 scores"
  - "tier_filter field on SearchFilter for downstream tier-based search filtering"
affects: [24-02-PLAN, 24-03-PLAN]

tech-stack:
  added: []
  patterns:
    - "TEXT NOT NULL DEFAULT with CHECK constraint for enum-like columns (avoids PG ENUM limitations)"
    - "Tier inference from write_path with caller override, mirroring trust_level inference pattern"
    - "JSONB array for provenance chain (source_ids), mirroring tags column pattern"

key-files:
  created:
    - crates/memcp-core/migrations/026_knowledge_tiers.sql
  modified:
    - crates/memcp-core/src/storage/store/mod.rs
    - crates/memcp-core/src/config.rs
    - crates/memcp-core/src/storage/store/postgres/queries.rs
    - crates/memcp-core/src/storage/store/postgres/mod.rs
    - crates/memcp-core/src/storage/store/postgres/embedding.rs
    - crates/memcp-core/src/storage/store/postgres/extraction.rs
    - crates/memcp-core/tests/knowledge_tiers_test.rs
    - crates/memcp-core/tests/common/builders.rs

key-decisions:
  - "Used TEXT NOT NULL DEFAULT with CHECK constraint for knowledge_tier (not PG ENUM) -- allows transactional migration and easy value additions"
  - "Tier inference follows existing trust_level inference pattern: unwrap_or_else on write_path match"
  - "source_ids stored as JSONB (matching tags column pattern) with partial GIN index on non-null rows"
  - "TierWeightsConfig nested inside SearchConfig for [search.tier_weights] TOML path"
  - "Backfill UPDATE uses CASE on write_path with ELSE 'explicit' for NULL/unknown paths"

requirements-completed: [TIER-01, TIER-02, TIER-03]

duration: 21min
completed: 2026-04-17
---

# Phase 24 Plan 01: Knowledge Tiers Schema, Structs, and Store-Layer Integration Summary

**Migration 026 with knowledge_tier (5-value TEXT CHECK) and source_ids (JSONB) columns, tier inference from write_path in store() with caller override, derived-tier source_ids validation, TierWeightsConfig for composite scoring weights, and tier_score_for() helper**

## Performance

- **Duration:** 21 min
- **Started:** 2026-04-17T22:55:03Z
- **Completed:** 2026-04-17T23:16:59Z
- **Tasks:** 2
- **Files modified:** 28 (14 per task commit)

## Accomplishments

- Created migration 026 adding `knowledge_tier` (TEXT NOT NULL DEFAULT 'explicit' CHECK), `source_ids` (JSONB), B-tree and GIN indexes, and backfill UPDATE by write_path
- Extended Memory struct with `knowledge_tier: String` and `source_ids: Option<Value>`
- Extended CreateMemory struct with optional `knowledge_tier` and `source_ids` fields
- Added `tier_filter` to SearchFilter for downstream tier-based filtering
- Added TierWeightsConfig (0.4/0.4/0.2 defaults) to SearchConfig
- Added `tier_score_for()` mapping tiers to 0.0-1.0 scores
- Implemented tier inference from write_path in store() with caller override
- Added derived-tier validation (rejects store without non-empty source_ids)
- Updated INSERT to bind knowledge_tier and source_ids columns
- Updated all SELECT lists across queries.rs, embedding.rs, extraction.rs (9 sites)
- Updated row_to_memory with knowledge_tier and source_ids extraction
- Wired MemoryBuilder build() to pass knowledge_tier and source_ids through
- Implemented and passed 5 Wave 1 tests (TIER-01, TIER-02, TIER-03)
- Threaded new fields through all 20+ CreateMemory/Memory/SearchFilter construction sites across codebase

## Task Commits

1. **Task 1: Migration 026 + Memory/CreateMemory/SearchFilter structs + TierWeightsConfig + tier_score_for()** - `8cd8a83` (feat)
2. **Task 2: Store-layer tier inference, source_ids validation, row_to_memory, SELECT list updates** - `ad6d2a3` (feat)

## Files Created/Modified

### Created
- `crates/memcp-core/migrations/026_knowledge_tiers.sql` -- knowledge_tier + source_ids schema migration with backfill

### Modified (core)
- `crates/memcp-core/src/storage/store/mod.rs` -- Memory, CreateMemory, SearchFilter struct extensions
- `crates/memcp-core/src/config.rs` -- TierWeightsConfig struct and tier_score_for() helper
- `crates/memcp-core/src/storage/store/postgres/queries.rs` -- tier inference, validation, INSERT, SELECT updates
- `crates/memcp-core/src/storage/store/postgres/mod.rs` -- row_to_memory extraction
- `crates/memcp-core/src/storage/store/postgres/embedding.rs` -- SELECT list updates
- `crates/memcp-core/src/storage/store/postgres/extraction.rs` -- SELECT list updates

### Modified (construction sites)
- `crates/memcp-core/src/transport/server.rs` -- MCP store handler
- `crates/memcp-core/src/transport/api/store.rs` -- HTTP store handler
- `crates/memcp-core/src/cli.rs` -- CLI store command
- `crates/memcp-core/src/pipeline/auto_store/mod.rs` -- auto-store worker (2 sites)
- `crates/memcp-core/src/pipeline/curation/worker.rs` -- curation merge + test helper
- `crates/memcp-core/src/intelligence/recall/mod.rs` -- queryless recall Memory construction
- `crates/memcp-core/src/benchmark/ingest.rs` -- benchmark ingest
- `crates/memcp-core/src/benchmark/locomo/ingest.rs` -- locomo benchmark (2 sites)

### Modified (tests)
- `crates/memcp-core/tests/knowledge_tiers_test.rs` -- 5 Wave 1 tests implemented
- `crates/memcp-core/tests/common/builders.rs` -- wired knowledge_tier/source_ids in build()
- `crates/memcp-core/tests/source_audit_test.rs` -- added new fields
- `crates/memcp-core/tests/import_test.rs` -- added new fields
- `crates/memcp-core/tests/journey_test.rs` -- added new fields
- `crates/memcp-core/tests/provenance_test.rs` -- added new fields
- `crates/memcp-core/tests/search_quality.rs` -- added new fields
- `crates/memcp-core/tests/curation_security_test.rs` -- added new fields + tier_filter
- `crates/memcp-core/tests/unit/salience.rs` -- added new fields to Memory construction
- `crates/memcp-core/tests/unit/temporal.rs` -- added new fields to Memory construction

## Decisions Made

- TEXT+CHECK constraint preferred over PG ENUM type (allows transactional migration, easy value additions -- confirmed by Phase 08.7 precedent)
- TierWeightsConfig nested inside SearchConfig (`[search.tier_weights]` TOML path) rather than top-level
- Backfill UPDATE uses CASE on write_path with ELSE 'explicit' to safely handle NULL/unknown paths
- tier_score_for() takes `&str` (not `Option<&str>`) since knowledge_tier is NOT NULL with a default

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed CreateMemory/Memory/SearchFilter construction across 20+ sites**
- **Found during:** Task 1 build verification
- **Issue:** Adding non-optional fields to Memory and optional fields to CreateMemory/SearchFilter broke all existing construction sites across the codebase (transport, pipeline, benchmarks, tests)
- **Fix:** Added `knowledge_tier: None, source_ids: None` to all CreateMemory sites, `knowledge_tier: "explicit".to_string(), source_ids: None` to all Memory sites, `tier_filter: None` to SearchFilter sites
- **Files modified:** 14 files across transport, pipeline, benchmark, and test modules
- **Committed in:** 8cd8a83 (Task 1) and ad6d2a3 (Task 2)

**2. [Rule 1 - Bug] Fixed test_tier_migration CHECK constraint test**
- **Found during:** Task 2 test execution
- **Issue:** Attempting UPDATE with invalid tier value aborted the sqlx::test transaction, causing subsequent valid-value UPDATEs to fail
- **Fix:** Used PL/pgSQL DO block with EXCEPTION handler to test CHECK constraint without aborting the transaction; restructured test to verify valid tiers via store() calls
- **Files modified:** crates/memcp-core/tests/knowledge_tiers_test.rs
- **Committed in:** ad6d2a3

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** All construction sites needed updating for compilation. Test restructuring was necessary for sqlx::test transaction semantics. No scope creep.

## Issues Encountered

### Pre-existing: test_tool_discovery count mismatch
- **File:** crates/memcp-core/tests/integration_test.rs:340
- **Issue:** Test expects 13 MCP tools but 16 exist. Prior phases added tools without updating the count.
- **Not caused by Phase 24** -- no MCP tools were added.
- **Logged to:** `.planning/phases/24-knowledge-tiers/deferred-items.md`

## User Setup Required
None.

## Next Phase Readiness
- Wave 1 foundation complete: migration applied, structs extended, store-layer inference working
- Wave 2 (24-02-PLAN): 4 tests to un-ignore (composite scoring, search ranking, tier filtering, queryless recall)
- Wave 3 (24-03-PLAN): 3 tests to un-ignore (source_ids roundtrip, show-sources, orphan tagging)

## Self-Check: PASSED

- [x] 026_knowledge_tiers.sql exists
- [x] 24-01-SUMMARY.md exists
- [x] Commit 8cd8a83 found
- [x] Commit ad6d2a3 found
