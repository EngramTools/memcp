---
phase: 20-test-quality-fixes
plan: 01
subsystem: testing
tags: [tracing-test, load-test, false-positive, deprecation-warning, test-quality]

requires:
  - phase: 10.2-load-test
    provides: SecurityReport type and load test binary
  - phase: 07.5-search-pagination
    provides: search_similar() with offset deprecation warning
provides:
  - Fixed test_offset_deprecation_warning to exercise actual tracing::warn code path
  - Wired false_positive_count computation in load test SecurityReport
  - New test_security_section_false_positives test
affects: []

tech-stack:
  added: [tracing-test 0.2.6 (dev-dependency, no-env-filter feature)]
  patterns: [traced_test macro for capturing tracing output in integration tests]

key-files:
  created: []
  modified:
    - crates/memcp-core/Cargo.toml
    - crates/memcp-core/tests/store_test.rs
    - crates/memcp-core/src/bin/load_test.rs
    - crates/memcp-core/src/load_test/report.rs

key-decisions:
  - "Used #[traced_test] macro with #[sqlx::test] — they compose correctly since sqlx::test only manages DB pool, not tracing subscriber"
  - "Enabled no-env-filter feature on tracing-test — required for integration tests (tests/ dir) to capture logs from the crate under test"

patterns-established:
  - "tracing assertion pattern: #[traced_test] + #[sqlx::test] for DB-backed tests that need to assert tracing output"

requirements-completed: [BENCH-SAFE-01, BENCH-SAFE-02, BENCH-SAFE-03, BENCH-SAFE-04]

duration: 10min
completed: 2026-03-10
---

# Phase 20 Plan 01: Test Quality Fixes Summary

**Fixed offset deprecation warning test to exercise search_similar() tracing::warn path, wired actual false_positive_count from quarantined-clean intersection in load test SecurityReport**

## Performance

- **Duration:** 10 min
- **Started:** 2026-03-10T03:19:03Z
- **Completed:** 2026-03-10T03:29:01Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- test_offset_deprecation_warning now calls search_similar() (not hybrid_search) and asserts "Offset-based search pagination is deprecated" via tracing-test
- false_positive_count in load test SecurityReport computed from quarantined intersect clean_ids instead of hardcoded 0
- make_security_report test helper accepts false_positive_count parameter; new test verifies rendering

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix test_offset_deprecation_warning** - `ff7110d` (fix)
2. **Task 2: Wire actual false_positive_count** - `1760e70` (fix)

## Files Created/Modified
- `crates/memcp-core/Cargo.toml` - Added tracing-test dev-dependency with no-env-filter
- `crates/memcp-core/tests/store_test.rs` - Rewrote test_offset_deprecation_warning to use search_similar() + traced_test
- `crates/memcp-core/src/bin/load_test.rs` - Computed false_positive_count from clean_set intersect quarantined
- `crates/memcp-core/src/load_test/report.rs` - Updated make_security_report helper, added false_positives rendering test

## Decisions Made
- Used `#[traced_test]` alongside `#[sqlx::test]` — they compose correctly since sqlx::test only manages the DB pool and does not install a tracing subscriber
- Enabled `no-env-filter` feature on tracing-test since integration tests in `tests/` are separate crates and need to capture logs from the library under test

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
- Pre-existing `test_tool_discovery` failure (expects 12 tools, finds 13) — not caused by this plan. Logged to `deferred-items.md`.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Test debt items closed; test names now match what they verify
- All security section tests pass (7 total)

---
*Phase: 20-test-quality-fixes*
*Completed: 2026-03-10*
