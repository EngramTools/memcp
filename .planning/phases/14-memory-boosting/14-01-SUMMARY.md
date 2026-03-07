---
phase: 14-memory-boosting
plan: 01
subsystem: api
tags: [mcp, uuid, integer-refs, hallucination-prevention, session-scoped]

# Dependency graph
requires: []
provides:
  - UuidRefMap struct: session-scoped integer ref mapping for memory IDs
  - ref field in all MCP tool responses containing memory IDs
  - Integer ref resolution in all ID-accepting MCP tools
affects: [14-02, 14-03, agents-using-mcp-tools]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Integer ref mapping: UuidRefMap assigns sequential integers (1, 2, 3...) to UUIDs per session"
    - "inject_ref helper: always injects ref alongside id in JSON response objects"
    - "resolve pattern: ref_map.resolve(&id).unwrap_or_else(|| id.clone()) for input handling"

key-files:
  created: []
  modified:
    - crates/memcp-core/src/transport/server.rs

key-decisions:
  - "UuidRefMap is session-scoped (one per MemoryService instance / per MCP connection) — refs reset between sessions"
  - "refs start at 1 not 0 — more natural for agents reading numbered lists"
  - "assign_ref is idempotent — same UUID always gets same integer within a session"
  - "resolve returns None only for unknown integer refs; UUID strings always pass through"
  - "recall_memory memories use memory_id field (not id) — ref injected explicitly alongside memory_id"
  - "inject_ref is called before field projection in search_memory — ref always present regardless of fields param"
  - "unwrap_or_else passthrough for unknown integer refs — store returns 'not found' naturally"

patterns-established:
  - "inject_ref pattern: call self.inject_ref(&mut obj) on any JSON Value containing an id field"
  - "resolve pattern: let id = self.ref_map.resolve(&params.id).unwrap_or_else(|| params.id.clone())"

requirements-completed: [UUID-01, UUID-02]

# Metrics
duration: 11min
completed: 2026-03-07
---

# Phase 14 Plan 01: UUID Hallucination Prevention Summary

**Session-scoped integer ref mapping (1, 2, 3...) added to all MCP tool responses and inputs via UuidRefMap struct in server.rs**

## Performance

- **Duration:** 11 min
- **Started:** 2026-03-07T05:52:05Z
- **Completed:** 2026-03-07T06:02:53Z
- **Tasks:** 2
- **Files modified:** 1

## Accomplishments
- UuidRefMap struct with idempotent assign_ref and resolve methods (both integer and UUID passthrough)
- ref field injected into all 8 MCP tools that return memory data (store/get/update/list/search/reinforce/annotate/recall)
- Integer ref resolution wired into all 6 ID-accepting tools (get/update/delete/reinforce/feedback/annotate)
- 7 unit tests: 5 for UuidRefMap core, 2 for inject_ref behavior on single objects and arrays

## Task Commits

Each task was committed atomically:

1. **Task 1: Create UuidRefMap struct with assign/resolve methods** - `bdae3cd` (feat)
2. **Task 2: Wire UUID ref mapping into all MCP tool responses and inputs** - `af78116` (feat)

**Plan metadata:** [pending final commit] (docs: complete plan)

## Files Created/Modified
- `crates/memcp-core/src/transport/server.rs` - UuidRefMap struct, inject_ref helper, ref_map field on MemoryService, ref wired into all tool responses/inputs

## Decisions Made
- UuidRefMap is session-scoped per MCP connection — refs reset naturally (no cleanup needed)
- refs start at 1 not 0 for agent readability
- recall_memory requires special handling since RecalledMemory uses memory_id not id field
- inject_ref called before field projection in search_memory so ref is always present
- Unknown integer refs fall through as-is (store returns "not found" naturally)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
- Pre-existing compilation errors in unrelated files (benchmark/runner.rs, import/openclaw.rs, config.rs) blocked `cargo test` for the full lib test binary. These are out-of-scope (pre-existing, not caused by this plan's changes). Tests were verified via targeted filter `transport::server::uuid_ref_tests` which compiled and ran cleanly.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- ref field now present on all MCP tool responses — agents can use integers instead of UUIDs
- UuidRefMap is in server.rs and reusable for future session-scoped features
- All ID-accepting tools resolve integer refs transparently — no agent-side changes needed

---
*Phase: 14-memory-boosting*
*Completed: 2026-03-07*

## Self-Check: PASSED
- File `crates/memcp-core/src/transport/server.rs` — confirmed contains `struct UuidRefMap` and `inject_ref`
- Commit `bdae3cd` — Task 1 (UuidRefMap struct)
- Commit `af78116` — Task 2 (ref wiring into all tools)
- All 7 uuid_ref_tests pass: `test result: ok. 7 passed; 0 failed`
