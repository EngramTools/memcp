---
phase: 16-test-coverage
plan: 01
subsystem: testing
tags: [unit-tests, salience, extraction, consolidation, tdd]

requires:
  - phase: 06-search
    provides: SalienceScorer, dedup_parent_chunks, extraction pipeline, consolidation pipeline
provides:
  - Unit tests for rank() ordering and score population
  - Unit tests for dedup_parent_chunks() parent/chunk deduplication
  - Unit tests for build_extraction_prompt()
  - Unit tests for build_synthesis_prompt() and concatenate_memories()
affects: [16-test-coverage]

tech-stack:
  added: []
  patterns: [make_scored_hit test helper for constructing ScoredHit with Memory]

key-files:
  created:
    - crates/memcp-core/tests/unit/extraction.rs
    - crates/memcp-core/tests/unit/consolidation.rs
  modified:
    - crates/memcp-core/tests/unit/salience.rs
    - crates/memcp-core/tests/unit.rs
    - crates/memcp-core/src/pipeline/consolidation/mod.rs

key-decisions:
  - "Made build_synthesis_prompt and concatenate_memories pub (were private fn) for test access from integration test harness"
  - "Used make_scored_hit helper with full Memory struct construction rather than MemoryBuilder (which builds CreateMemory, not Memory)"

patterns-established:
  - "make_scored_hit(): reusable helper for ScoredHit construction in salience tests"

requirements-completed: [P1-1, P1-1b, P2-7]

duration: 6min
completed: 2026-03-09
---

# Phase 16 Plan 01: Pure-Logic Unit Test Coverage Summary

**15 new unit tests covering SalienceScorer::rank(), dedup_parent_chunks(), build_extraction_prompt(), build_synthesis_prompt(), and concatenate_memories()**

## Performance

- **Duration:** 6 min
- **Started:** 2026-03-09T20:41:01Z
- **Completed:** 2026-03-09T20:47:00Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- 8 new tests for rank() and dedup_parent_chunks() covering ordering, empty input, score population, and parent/chunk deduplication
- 7 new tests for extraction prompt building and consolidation synthesis/concatenation functions
- All 86 unit tests pass (71 existing + 15 new), zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Add rank() and dedup_parent_chunks() tests** - `0f19e2f` (test)
2. **Task 2: Add extraction and consolidation pure-function tests** - `ab8a851` (test)

## Files Created/Modified
- `crates/memcp-core/tests/unit/salience.rs` - Added make_scored_hit helper + 8 tests for rank() and dedup_parent_chunks()
- `crates/memcp-core/tests/unit/extraction.rs` - 3 tests for build_extraction_prompt()
- `crates/memcp-core/tests/unit/consolidation.rs` - 4 tests for build_synthesis_prompt() and concatenate_memories()
- `crates/memcp-core/tests/unit.rs` - Registered extraction and consolidation modules
- `crates/memcp-core/src/pipeline/consolidation/mod.rs` - Made build_synthesis_prompt and concatenate_memories pub

## Decisions Made
- Made consolidation helper functions `pub` instead of `pub(crate)` since integration tests are external crates and can't access `pub(crate)` items
- Constructed Memory structs directly rather than using MemoryBuilder (which produces CreateMemory, not Memory)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed test_rank_populates_scores assertion**
- **Found during:** Task 1 (TDD GREEN phase)
- **Issue:** Test asserted `salience_score > 0.0` for all hits, but min-max normalization gives the worst hit 0.0 in every dimension
- **Fix:** Changed assertion to check best hit > 0.0 and worst hit >= 0.0
- **Files modified:** crates/memcp-core/tests/unit/salience.rs
- **Verification:** All 19 salience tests pass

---

**Total deviations:** 1 auto-fixed (1 bug in test logic)
**Impact on plan:** Minor test assertion correction. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Pure-logic functions now have test coverage
- Ready for 16-02 (integration test coverage if applicable)

---
*Phase: 16-test-coverage*
*Completed: 2026-03-09*
