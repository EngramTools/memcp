---
phase: 21-pii-and-secret-redaction
plan: 02
subsystem: pipeline
tags: [redaction, ingestion, security, cli, mcp, http-api, auto-store]

requires:
  - phase: 21-pii-and-secret-redaction
    provides: "RedactionEngine with two-phase RegexSet scan, 13 secret patterns, PII masking, entropy filter"
provides:
  - "Redaction wired into all four ingestion paths (CLI, HTTP API, MCP, auto-store)"
  - "CLI --no-redact bypass flag"
  - "HTTP API and MCP skip_redaction bypass parameter"
  - "Auto-store mandatory redaction (no bypass)"
  - "Store response redaction metadata (count + categories)"
  - "10 integration tests for redaction behavior"
affects: [production-hardening, engram-hosting]

tech-stack:
  added: []
  patterns: ["fail-closed redaction (reject store on error)", "per-path bypass controls (CLI flag, API param, auto-store always-on)"]

key-files:
  created:
    - crates/memcp-core/tests/redaction_integration.rs
  modified:
    - crates/memcp-core/src/transport/daemon.rs
    - crates/memcp-core/src/transport/api/store.rs
    - crates/memcp-core/src/transport/api/types.rs
    - crates/memcp-core/src/transport/server.rs
    - crates/memcp-core/src/transport/health/mod.rs
    - crates/memcp-core/src/pipeline/auto_store/mod.rs
    - crates/memcp-core/src/cli.rs
    - crates/memcp/src/main.rs
    - crates/memcp-core/src/lib.rs

key-decisions:
  - "RedactionEngine constructed early in daemon startup (before health server) so AppState has it available"
  - "Auto-store always redacts with no bypass — auto-ingested content is untrusted"
  - "CLI constructs engine per-invocation (cheap, acceptable for CLI latency)"

patterns-established:
  - "Redaction before content_filter before embedding: consistent ordering across all paths"
  - "Skip bypass is per-path: CLI uses --no-redact, API/MCP use skip_redaction param, auto-store has no bypass"

requirements-completed: [RED-08, RED-09, RED-10, RED-11, RED-12, RED-13]

duration: 14min
completed: 2026-03-10
---

# Phase 21 Plan 02: Ingestion Integration Summary

**RedactionEngine wired into all four ingestion paths with per-path bypass controls, fail-closed error handling, response metadata, and 10 integration tests**

## Performance

- **Duration:** 14 min
- **Started:** 2026-03-10T08:05:03Z
- **Completed:** 2026-03-10T08:19:03Z
- **Tasks:** 2
- **Files modified:** 13

## Accomplishments
- Daemon constructs RedactionEngine at startup, fail-closed when secrets_enabled=true
- HTTP API and MCP server apply redaction before content_filter with skip_redaction bypass
- CLI store applies redaction with --no-redact bypass flag
- Auto-store worker always redacts (no bypass), fail-closed on error (skips memory)
- Store responses include redaction metadata (count + categories) when content was redacted
- 10 integration tests covering: secret redaction, PII opt-in, bypass controls, multiple secrets, fail-closed behavior, allowlist, metadata correctness

## Task Commits

Each task was committed atomically:

1. **Task 1: Daemon construction + HTTP API + MCP wiring** - `7378838` (feat)
2. **Task 2: CLI + auto-store wiring + integration tests** - `da9839e` (feat)

## Files Created/Modified
- `crates/memcp-core/src/transport/daemon.rs` - RedactionEngine construction at startup, passed to auto-store and AppState
- `crates/memcp-core/src/transport/api/store.rs` - Redaction before temporal extraction, metadata in response
- `crates/memcp-core/src/transport/api/types.rs` - RedactionInfo struct, skip_redaction field on StoreRequest
- `crates/memcp-core/src/transport/server.rs` - Redaction in MCP store_memory before content_filter, skip_redaction param
- `crates/memcp-core/src/transport/health/mod.rs` - redaction_engine field on AppState
- `crates/memcp-core/src/pipeline/auto_store/mod.rs` - Mandatory redaction before content_filter, uses redacted content downstream
- `crates/memcp-core/src/cli.rs` - Redaction in cmd_store with no_redact bypass
- `crates/memcp/src/main.rs` - --no-redact CLI flag, redaction engine construction in serve path
- `crates/memcp-core/src/lib.rs` - Re-export pipeline::redaction
- `crates/memcp-core/tests/redaction_integration.rs` - 10 integration tests
- `crates/memcp-core/src/bin/load_test.rs` - AppState field fix
- `crates/memcp-core/tests/api_test.rs` - AppState field fix
- `crates/memcp-core/tests/rate_limit_test.rs` - AppState field fix
- `crates/memcp-core/tests/trust_retrieval_test.rs` - AppState field fix
- `crates/memcp-core/tests/metrics_test.rs` - AppState field fix

## Decisions Made
- RedactionEngine constructed early in daemon startup (before health server) so it is available in AppState for HTTP API handlers
- Auto-store has no bypass mechanism — auto-ingested content from session logs is untrusted and must always be redacted
- CLI constructs engine per-invocation rather than sharing from daemon (simple, fast enough for CLI use)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed AppState construction in test files**
- **Found during:** Task 1 (build verification)
- **Issue:** Adding redaction_engine field to AppState broke load_test.rs and 4 test files (api_test, rate_limit_test, trust_retrieval_test, metrics_test) that construct AppState
- **Fix:** Added `redaction_engine: None` to all AppState instances in test code
- **Files modified:** crates/memcp-core/src/bin/load_test.rs, crates/memcp-core/tests/api_test.rs, rate_limit_test.rs, trust_retrieval_test.rs, metrics_test.rs
- **Verification:** All files compile, existing tests unaffected
- **Committed in:** 7378838 (Task 1) and da9839e (Task 2)

**2. [Rule 1 - Bug] Fixed redaction engine construction ordering in main.rs serve path**
- **Found during:** Task 2 (build verification)
- **Issue:** redaction_engine was constructed after auto-store spawn, but auto-store needs it
- **Fix:** Moved construction to before auto-store spawn (step 8b2, before 8c)
- **Files modified:** crates/memcp/src/main.rs
- **Verification:** Binary compiles and links correctly

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes necessary for compilation. No scope creep.

## Issues Encountered
None beyond the auto-fixed items documented above.

## User Setup Required
None - redaction is enabled by default with no external service dependencies.

## Next Phase Readiness
- All redaction features are active on default config (secrets_enabled=true)
- Phase 21 (PII and secret redaction) is complete
- Ready for production deployment

---
*Phase: 21-pii-and-secret-redaction*
*Completed: 2026-03-10*
