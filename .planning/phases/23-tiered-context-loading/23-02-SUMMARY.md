---
phase: 23-tiered-context-loading
plan: 02
subsystem: pipeline
tags: [abstraction, embedding, background-worker, postgres, metrics]

requires:
  - phase: 23-01
    provides: "AbstractionProvider trait, Ollama/OpenAI impls, AbstractionConfig, abstract_text/overview_text/abstraction_status schema columns"

provides:
  - "Background abstraction worker polling DB every 5s for pending memories"
  - "Embedding pipeline now embeds against abstract_text (L0) when available"
  - "Race prevention: embedding blocked on memories with abstraction_status='pending'"
  - "Abstraction worker spawned in daemon step 3.8, before embedding pipeline"
  - "Metrics: memcp_abstraction_jobs_total{status} counter + memcp_abstraction_duration_seconds histogram"
  - "get_pending_abstractions, update_abstraction_fields, update_abstraction_status on PostgresMemoryStore"
  - "TCL-02 tests: test_embed_uses_abstract_text and test_embed_falls_back_to_content"

affects:
  - 23-03
  - search-quality
  - embedding-pipeline

tech-stack:
  added: []
  patterns:
    - "Abstraction-before-embedding ordering: worker spawned in step 3.8, before embedding pipeline step 4"
    - "Fail-open abstraction: LLM errors set status='failed', memory still embeds against full content"
    - "Race prevention via DB filter: get_pending_memories adds AND abstraction_status != 'pending'"

key-files:
  created:
    - crates/memcp-core/src/pipeline/abstraction/worker.rs
  modified:
    - crates/memcp-core/src/pipeline/abstraction/mod.rs
    - crates/memcp-core/src/storage/store/postgres.rs
    - crates/memcp-core/src/transport/daemon.rs
    - crates/memcp-core/src/intelligence/embedding/mod.rs
    - crates/memcp-core/src/intelligence/embedding/pipeline.rs
    - crates/memcp-core/src/transport/server.rs
    - crates/memcp-core/src/transport/api/store.rs
    - crates/memcp-core/src/pipeline/auto_store/mod.rs
    - crates/memcp-core/src/pipeline/promotion/worker.rs
    - crates/memcp-core/src/benchmark/ingest.rs
    - crates/memcp-core/src/benchmark/locomo/ingest.rs
    - crates/memcp-core/tests/unit/abstraction.rs

key-decisions:
  - "build_embedding_text signature extended with abstract_text: Option<&str> param — falls back to content when None"
  - "Race guard placed in get_pending_memories SQL (AND abstraction_status != 'pending') rather than application layer — single enforcement point"
  - "L1 overview failure is non-fatal: worker continues with L0 only and logs warning rather than marking failed"
  - "Abstraction worker spawned at step 3.8 in daemon, one step before embedding pipeline (step 4), ensuring ordering"

requirements-completed: [TCL-01, TCL-02]

duration: 35min
completed: 2026-03-12
---

# Phase 23 Plan 02: Abstraction Worker + Embedding Pipeline Summary

**Background abstraction worker generating L0 abstracts from LLM, embedding pipeline preferring abstract_text over full content with race prevention via SQL filter**

## Performance

- **Duration:** ~35 min
- **Started:** 2026-03-12T21:30:00Z
- **Completed:** 2026-03-12T22:05:00Z
- **Tasks:** 2
- **Files modified:** 12

## Accomplishments

- Abstraction worker polls DB every 5s, processes pending memories in batches of 50, marks complete/failed/skipped
- `build_embedding_text` updated with `abstract_text: Option<&str>` — prefers L0 abstract for better semantic search quality
- Race condition prevented: `get_pending_memories` filters out `abstraction_status='pending'` so embedding waits for abstraction
- Daemon wires abstraction worker at step 3.8 (before embedding pipeline step 4), ensuring correct ordering
- 3 new PostgresMemoryStore methods: `get_pending_abstractions`, `update_abstraction_fields`, `update_abstraction_status`
- 11 call sites updated across the codebase to pass `abstract_text` argument
- TCL-02 unit tests passing: `test_embed_uses_abstract_text` and `test_embed_falls_back_to_content`
- 116 unit tests pass (3 Plan 03 depth tests remain ignored)

## Task Commits

1. **Task 1: Abstraction worker + daemon wiring** - `65857be` (feat)
2. **Task 2: Embedding pipeline uses abstract_text + TCL-02 tests** - `223c298` (feat)

## Files Created/Modified

- `crates/memcp-core/src/pipeline/abstraction/worker.rs` - New background worker: 5s poll loop, L0/L1 generation, fail-open, metrics
- `crates/memcp-core/src/pipeline/abstraction/mod.rs` - Added `pub mod worker`
- `crates/memcp-core/src/storage/store/postgres.rs` - 3 new abstraction methods + race guard in get_pending_memories
- `crates/memcp-core/src/transport/daemon.rs` - Step 3.8 spawns abstraction worker before embedding pipeline; updated build_embedding_text call
- `crates/memcp-core/src/intelligence/embedding/mod.rs` - build_embedding_text signature updated (abstract_text param)
- `crates/memcp-core/src/intelligence/embedding/pipeline.rs` - backfill uses abstract_text
- `crates/memcp-core/src/transport/server.rs` - 2 call sites updated
- `crates/memcp-core/src/transport/api/store.rs` - call site updated
- `crates/memcp-core/src/pipeline/auto_store/mod.rs` - 2 call sites updated
- `crates/memcp-core/src/pipeline/promotion/worker.rs` - call site updated
- `crates/memcp-core/src/benchmark/ingest.rs` - call site updated
- `crates/memcp-core/src/benchmark/locomo/ingest.rs` - 2 call sites updated
- `crates/memcp-core/tests/unit/abstraction.rs` - TCL-02 tests implemented and un-ignored

## Decisions Made

- `build_embedding_text` signature change is additive (new param) applied to all 11 callers — consistent behavior, single source of truth
- Race guard placed in SQL (`AND abstraction_status != 'pending'`) rather than application layer — cleaner, single enforcement point
- L1 overview failure treated as non-fatal — L0 abstract alone improves embedding quality; degrading gracefully is better than marking the whole entry failed
- `abstraction_status='failed'` memories are still eligible for embedding (via fall-through to content) — fail-open preserves functionality

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None. One missed call site (locomo/ingest.rs line 141 had different indentation than line 80, requiring a second edit) but caught by `cargo build` immediately.

## Next Phase Readiness

- Plan 23-03 can implement the `depth` parameter for tiered context loading (TCL-05/TCL-06 tests await)
- Abstraction backfill on daemon startup not yet implemented — Plan 23-03 or later can add startup sweep
- `count_pending_abstractions` already available for /status endpoint integration

---
*Phase: 23-tiered-context-loading*
*Completed: 2026-03-12*
