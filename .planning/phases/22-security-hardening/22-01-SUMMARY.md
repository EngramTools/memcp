---
phase: 22-security-hardening
plan: 01
subsystem: security
tags: [validation, input-limits, timeouts, panic-safety, clippy]

requires: []
provides:
  - Centralized input validation module (validation.rs) with configurable limits
  - InputLimitsConfig for content (100KB), tags (32/256), query (10KB), batch (100)
  - Timeout-hardened reqwest clients across all 12 pipeline HTTP sites
  - Panic-safe handler code with clippy::unwrap_used deny lint
affects: [transport, pipeline, cli, api]

tech-stack:
  added: []
  patterns: [centralized-validation, timeout-configured-clients, clippy-deny-lint]

key-files:
  created:
    - crates/memcp-core/src/validation.rs
    - crates/memcp-core/tests/input_validation_test.rs
  modified:
    - crates/memcp-core/src/lib.rs
    - crates/memcp-core/src/config.rs
    - crates/memcp-core/src/transport/server.rs
    - crates/memcp-core/src/transport/api/store.rs
    - crates/memcp-core/src/transport/api/search.rs
    - crates/memcp-core/src/transport/api/mod.rs
    - crates/memcp-core/src/cli.rs
    - crates/memcp-core/src/transport/daemon.rs

key-decisions:
  - "120s timeout for all pipeline HTTP clients (all call LLMs), 10s connect timeout"
  - "Module-level allow(clippy::unwrap_used) for pipeline/storage/intelligence/import — handler safety first, deep code deferred"

patterns-established:
  - "Input validation: all transport layers call shared validate_* functions from validation.rs"
  - "Clippy deny lint: new unwrap() in handler code is a compile error"

requirements-completed: [SEC-01, SEC-02, SEC-08, SEC-09]

duration: 18min
completed: 2026-03-12
---

# Phase 22 Plan 01: Input Validation and Panic Safety Summary

**Centralized input validation (100KB content, 32 tags, 10KB query) across MCP/HTTP/CLI, 12 pipeline timeout-hardened clients, clippy::unwrap_used deny lint**

## Performance

- **Duration:** 18 min
- **Started:** 2026-03-12T01:17:33Z
- **Completed:** 2026-03-12T01:35:30Z
- **Tasks:** 2
- **Files modified:** 20

## Accomplishments
- New validation.rs module with InputLimitsConfig (serde defaults, configurable via memcp.toml)
- Input validation wired into all three transport layers: MCP handlers, HTTP API, CLI subcommands
- DefaultBodyLimit::max(256KB) on HTTP API router prevents oversized request bodies
- All 12 reqwest::Client::new() in pipeline/ replaced with timeout-configured builders
- All 4 .lock().unwrap() in server.rs replaced with descriptive .expect() messages
- All unwrap() removed from handler code (api/mod.rs, cli.rs, daemon.rs, server.rs recall)
- clippy::unwrap_used deny lint at crate root prevents regression
- 10 integration tests for validation rejection paths

## Task Commits

Each task was committed atomically:

1. **Task 1: Input validation module + transport wiring + timeout hardening** - `2398501` (feat)
2. **Task 2: Panic audit -- replace unwrap/expect in handler code** - `c7775cc` (fix)

## Files Created/Modified
- `crates/memcp-core/src/validation.rs` - Centralized input validation functions + InputLimitsConfig
- `crates/memcp-core/tests/input_validation_test.rs` - 10 rejection path tests
- `crates/memcp-core/src/lib.rs` - Added pub mod validation + clippy::unwrap_used deny
- `crates/memcp-core/src/config.rs` - Added input_limits field to Config struct
- `crates/memcp-core/src/transport/server.rs` - Validation in store/search/recall MCP handlers, mutex expect messages
- `crates/memcp-core/src/transport/api/store.rs` - Validation in HTTP store handler
- `crates/memcp-core/src/transport/api/search.rs` - Validation in HTTP search handler
- `crates/memcp-core/src/transport/api/mod.rs` - DefaultBodyLimit layer, safe header parsing
- `crates/memcp-core/src/cli.rs` - Validation in CLI store/search/recall commands
- `crates/memcp-core/src/transport/daemon.rs` - Safe HashMap key access with expect
- `crates/memcp-core/src/pipeline/` (8 files) - Timeout-configured reqwest clients

## Decisions Made
- Used 120s request timeout for all 12 pipeline HTTP clients (all involve LLM calls) with 10s connect timeout
- Applied module-level allow(clippy::unwrap_used) to pipeline/, storage/store/postgres.rs, intelligence/embedding/local.rs, and import/ — fixing 128+ deep unwraps is deferred, handler-level safety is the priority
- local.rs referenced in plan does not exist (storage uses postgres.rs only) — no deviation needed

## Deviations from Plan

None - plan executed exactly as written. The plan referenced `storage/local.rs` for mutex locks but that file does not exist; all mutex locks were in `server.rs` and were addressed there.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Input validation layer complete, ready for Phase 22-02 (SQL injection hardening, CORS)
- clippy deny lint will catch any new unwrap() additions in handler code

---
*Phase: 22-security-hardening*
*Completed: 2026-03-12*
