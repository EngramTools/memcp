---
phase: 24-knowledge-tiers
plan: "02"
subsystem: search
tags: [rust, composite-score, tier-filter, knowledge-tier, recall, sqlx]

requires:
  - phase: 24-knowledge-tiers
    plan: "01"
    provides: "Migration 026, Memory/CreateMemory structs with knowledge_tier/source_ids, TierWeightsConfig, tier_score_for()"
provides:
  - "3-dimensional composite score formula (RRF + salience*trust + tier) in HTTP, MCP, and CLI paths"
  - "Tier filter SQL in search_similar() and recall_candidates() — excludes raw by default (D-10)"
  - "Queryless recall returns all tiers including raw (D-11)"
  - "4 passing tests for TIER-04 and TIER-06 requirements"
affects: [24-03-PLAN]

tech-stack:
  added: []
  patterns:
    - "Tier filter in SQL WHERE clause via knowledge_tier != 'raw' default, ANY($N) for explicit list, no clause for 'all'"
    - "Composite score uses configurable weights from TierWeightsConfig (w_rrf, w_sal, w_tier)"

key-files:
  created: []
  modified:
    - crates/memcp-core/src/storage/store/postgres/embedding.rs
    - crates/memcp-core/src/storage/store/postgres/salience.rs
    - crates/memcp-core/src/transport/api/search.rs
    - crates/memcp-core/src/transport/server.rs
    - crates/memcp-core/src/cli.rs
    - crates/memcp-core/src/intelligence/recall/mod.rs
    - crates/memcp-core/tests/knowledge_tiers_test.rs

key-decisions:
  - "Tier filter applied in SQL WHERE clause, not post-fetch — efficient at database level"
  - "CLI uses hardcoded weights (0.4/0.4/0.2) since it lacks full config access; HTTP and MCP use TierWeightsConfig from config"
  - "recall_candidates() gains tier_filter parameter; recall_candidates_queryless() unchanged (D-11 compliance)"
  - "test_tier_search_ranking uses direct composite score math rather than full search pipeline — validates scoring logic without embedding infrastructure"

patterns-established:
  - "Tier filter three-way match: None = exclude raw, Some(['all']) = no filter, Some([list]) = exact list"

requirements-completed: [TIER-04, TIER-06]

duration: 14min
completed: 2026-04-17
---

# Phase 24 Plan 02: Tier Scoring and Filtering Summary

**3-dimensional composite score (RRF + salience*trust + tier_score) in all 3 search paths, SQL-level tier filtering excluding raw by default, queryless recall returning all tiers**

## Performance

- **Duration:** 14 min
- **Started:** 2026-04-17T23:19:45Z
- **Completed:** 2026-04-17T23:33:54Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments

- Updated composite score formula in all 3 paths (HTTP, MCP, CLI) to include tier dimension: `w_rrf * RRF + w_sal * (salience * trust) + w_tier * tier_score`
- Added tier filter to embedding.rs `search_similar()` WHERE clause — excludes raw by default (D-10), supports explicit tier list and "all" bypass
- Added tier filter to salience.rs `recall_candidates()` with new `tier_filter: Option<&[String]>` parameter; `recall_candidates_queryless()` unchanged per D-11
- Implemented and passed 4 Wave 2 tests: composite score tier boost, search ranking, tier filter excludes raw, queryless recall all tiers
- Fixed missing `knowledge_tier, source_ids` in `discover_associations` SELECT lists (Rule 1 bug fix)

## Task Commits

1. **Task 1: Tier filter SQL + composite score in all 3 paths** - `10ae84a` (feat)
2. **Task 2: TIER-04/06 tests** - `dae3d0c` (test)

## Files Created/Modified

- `crates/memcp-core/src/storage/store/postgres/embedding.rs` -- tier filter in search_similar() WHERE clause + discover_associations SELECT fix
- `crates/memcp-core/src/storage/store/postgres/salience.rs` -- tier_filter param on recall_candidates() and recall_candidates_multi_tier()
- `crates/memcp-core/src/transport/api/search.rs` -- composite score with tier dimension using config weights
- `crates/memcp-core/src/transport/server.rs` -- MCP composite score with tier dimension using search_config.tier_weights
- `crates/memcp-core/src/cli.rs` -- CLI composite score with hardcoded 0.4/0.4/0.2 weights
- `crates/memcp-core/src/intelligence/recall/mod.rs` -- passes None tier_filter for query-based recall (D-10 default)
- `crates/memcp-core/tests/knowledge_tiers_test.rs` -- 4 Wave 2 tests implemented and passing

## Decisions Made

- Tier filter applied in SQL WHERE clause, not post-fetch, for performance
- CLI uses hardcoded weights (0.4/0.4/0.2) since it runs locally without full config; HTTP and MCP paths use TierWeightsConfig from config
- test_tier_search_ranking validates scoring math directly via composite score calculation rather than requiring embedding pipeline in test

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed missing knowledge_tier, source_ids in discover_associations SELECT lists**
- **Found during:** Task 1 (embedding.rs changes)
- **Issue:** Both project-scoped and non-project-scoped `discover_associations` queries were missing `m.knowledge_tier, m.source_ids` in SELECT, causing row_to_memory to silently fall back to defaults
- **Fix:** Added `m.knowledge_tier, m.source_ids` to both SELECT lists
- **Files modified:** crates/memcp-core/src/storage/store/postgres/embedding.rs
- **Committed in:** 10ae84a (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug fix)
**Impact on plan:** Prevents silent wrong-tier data from discover_associations queries. No scope creep.

## Issues Encountered

### Pre-existing: test_tool_discovery count mismatch
- **File:** crates/memcp-core/tests/integration_test.rs:340
- **Issue:** Test expects 13 MCP tools but 16 exist. Prior phases added tools without updating the count.
- **Not caused by Phase 24** -- no MCP tools were added.
- **Already logged to:** `.planning/phases/24-knowledge-tiers/deferred-items.md` (Plan 01)

## User Setup Required
None.

## Next Phase Readiness
- Wave 2 complete: tier scoring and filtering fully operational
- Wave 3 (24-03-PLAN): 3 tests to un-ignore (source_ids roundtrip, show-sources, orphan tagging)
- 9 of 12 knowledge_tiers tests now pass, 3 remain ignored for Wave 3

## Self-Check: PASSED

- [x] 24-02-SUMMARY.md exists
- [x] Commit 10ae84a found
- [x] Commit dae3d0c found
- [x] All 7 modified files exist
- [x] 9 tests pass in knowledge_tiers_test.rs (4 new + 5 from Wave 1)

---
*Phase: 24-knowledge-tiers*
*Completed: 2026-04-17*
