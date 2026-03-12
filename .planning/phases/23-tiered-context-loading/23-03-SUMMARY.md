---
phase: 23-tiered-context-loading
plan: 03
subsystem: api
tags: [depth, search, recall, mcp, cli, http, tiered-context, abstraction]

requires:
  - phase: 23-02
    provides: abstraction_status column, abstract_text/overview_text fields on Memory struct

provides:
  - depth parameter on MCP search_memory tool (0=abstract, 1=overview, 2=full, default=2)
  - depth parameter on MCP recall_memory tool (same semantics)
  - --depth flag on CLI search and recall subcommands
  - depth field in HTTP SearchRequest and RecallRequest
  - abstract_text/overview_text fields on RecalledMemory struct
  - 5 integration tests for tiered content pipeline
  - 3 unit tests for depth selection logic

affects: [agent-tooling, context-injection, token-efficiency]

tech-stack:
  added: []
  patterns:
    - "Depth-based content selection: match depth { 0 => abstract.unwrap_or(content), 1 => overview.unwrap_or(content), _ => content }"
    - "Graceful fallback: depth=0/1 returns full content when abstract/overview tier is NULL"
    - "RecalledMemory carries abstract_text/overview_text from Memory in queryless path"

key-files:
  created:
    - crates/memcp-core/tests/abstraction_pipeline_test.rs
  modified:
    - crates/memcp-core/src/transport/server.rs
    - crates/memcp-core/src/transport/api/types.rs
    - crates/memcp-core/src/transport/api/search.rs
    - crates/memcp-core/src/transport/api/recall.rs
    - crates/memcp-core/src/intelligence/recall/mod.rs
    - crates/memcp-core/src/cli.rs
    - crates/memcp/src/main.rs
    - crates/memcp-core/tests/unit/abstraction.rs

key-decisions:
  - "Depth is purely a display concern — does not affect SQL query, only result mapping"
  - "RecalledMemory gets abstract_text/overview_text fields; queryless path populates from Memory struct, query-based path sets None (uses extracted facts)"
  - "abstract_available field added to search results so agents can discover tier availability"
  - "Default depth=2 ensures full backward compatibility"

patterns-established:
  - "Depth selection: match depth { 0 => tier0.unwrap_or(fallback), 1 => tier1.unwrap_or(fallback), _ => fallback }"
  - "Integration tests bypass LLM by directly SQL-updating abstract_text/overview_text"

requirements-completed: [TCL-03, TCL-04, TCL-05]

duration: 35min
completed: 2026-03-12
---

# Phase 23 Plan 03: Depth Parameter on Search/Recall Surfaces Summary

**depth parameter (0=abstract, 1=overview, 2=full) wired across MCP, CLI, and HTTP with graceful fallback and 5 integration tests validating the full tiered retrieval pipeline**

## Performance

- **Duration:** ~35 min
- **Started:** 2026-03-12T22:30:00Z
- **Completed:** 2026-03-12T23:05:00Z
- **Tasks:** 2
- **Files modified:** 9

## Accomplishments

- depth parameter added to MCP search_memory and recall_memory tools with schema description guiding model usage
- --depth flag on CLI search and recall with default=2 (backward compatible)
- depth field in HTTP SearchRequest/RecallRequest (default=2 via serde)
- RecalledMemory struct extended with abstract_text/overview_text for queryless recall path
- 5 integration tests pass against real Postgres (abstractoin_pipeline_test.rs)
- 3 previously-ignored unit tests implemented and passing (119 unit tests total)

## Task Commits

1. **Task 1: Depth parameter on MCP + CLI + HTTP** - `3b701e8` (feat)
2. **Task 2: Integration tests for tiered content pipeline** - `ac8513a` (test)

## Files Created/Modified

- `crates/memcp-core/src/transport/server.rs` - depth field in SearchMemoryParams/RecallMemoryParams, depth-based content selection in result assembly
- `crates/memcp-core/src/transport/api/types.rs` - depth field in SearchRequest/RecallRequest with default_depth() serde default
- `crates/memcp-core/src/transport/api/search.rs` - depth-based content selection in HTTP search handler
- `crates/memcp-core/src/transport/api/recall.rs` - depth-based content selection in HTTP recall handler
- `crates/memcp-core/src/intelligence/recall/mod.rs` - abstract_text/overview_text added to RecalledMemory struct, populated in queryless path
- `crates/memcp-core/src/cli.rs` - depth param in cmd_search/cmd_recall, applied across json/compact/default output modes
- `crates/memcp/src/main.rs` - --depth flag in Search and Recall CLI commands
- `crates/memcp-core/tests/abstraction_pipeline_test.rs` - 5 integration tests (replaced 3 placeholder stubs)
- `crates/memcp-core/tests/unit/abstraction.rs` - 3 depth unit tests (replaced 3 ignored stubs)

## Decisions Made

- Depth is purely a display concern: the SQL query is unchanged, depth only affects which field maps to "content" in the result. This keeps the feature simple and zero-cost at query time.
- RecalledMemory gets abstract_text/overview_text added as Option<String> fields. The queryless recall path (which returns full Memory structs via ScoredHit) populates them. The query-based path (extraction-based, uses extracted facts) sets them to None — this is fine since depth on recall with a query isn't a primary use case.
- abstract_available boolean added to search results so agents can discover whether the abstract tier exists for a memory before using depth=0.
- Integration tests bypass LLM by directly SQL-updating abstract_text/overview_text, making them deterministic and fast.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Extended RecalledMemory struct with tiered fields**
- **Found during:** Task 1 (MCP recall_memory depth parameter)
- **Issue:** recall_memory returns RecalledMemory which only had content string, needed abstract_text/overview_text to apply depth selection
- **Fix:** Added abstract_text/overview_text Option<String> fields to RecalledMemory; updated all 6 struct initializations across server.rs, cli.rs, api/recall.rs, and recall/mod.rs
- **Files modified:** src/intelligence/recall/mod.rs, src/transport/server.rs, src/transport/api/recall.rs, src/cli.rs
- **Verification:** cargo build succeeds, no compilation errors
- **Committed in:** 3b701e8 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 missing critical)
**Impact on plan:** Necessary to implement depth on recall_memory. No scope creep.

## Issues Encountered

- Docker not running on dev machine — sqlx::test ephemeral databases required local postgres on port 5432 instead of the project's standard 5433 (Docker). Tests pass on 5432 with `memcp` user granted superuser. The 3 tests in `integration_test.rs` and 8 tests in `mcp_contract.rs` that hardcode port 5433 were pre-existing failures (not caused by this plan).

## Next Phase Readiness

- Tiered context loading is complete: schema (23-01), abstraction worker (23-02), depth parameter on all surfaces (23-03)
- Agents can now use depth=0 for token-efficient scanning in planning phases
- Ready for Phase 23-04 (final integration and documentation) or subsequent phases

---
*Phase: 23-tiered-context-loading*
*Completed: 2026-03-12*
