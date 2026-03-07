---
phase: 10-production-hardening
plan: 03
subsystem: infra
tags: [prometheus, metrics, observability, gc, embedding, search, recall, dedup]

# Dependency graph
requires:
  - phase: 10-01
    provides: metrics recorder installed, metric names described, metrics macros available globally

provides:
  - GC worker counter increments (memcp_gc_runs_total, memcp_gc_pruned_total)
  - Dedup worker counter increments (memcp_dedup_merges_total)
  - Embedding pipeline counters + duration histogram (memcp_embedding_jobs_total, memcp_embedding_duration_seconds)
  - Memory count gauges updated per embedding job (memcp_memories_total, memcp_memories_pending_embedding)
  - Recall handler histogram (memcp_recall_memories_returned)
  - Search handler histogram (memcp_search_results_returned, both paths including empty-result early return)

affects:
  - 10-05
  - any Prometheus dashboards or alerting rules consuming memcp_* metrics

# Tech tracking
tech-stack:
  added: []
  patterns:
    - metrics::counter!/histogram!/gauge! macros called at actual execution points (not just describe_counter! registration)
    - Timing pattern: std::time::Instant::now() + elapsed().as_secs_f64() wrapping embed call
    - Gauge update pattern: query store after each batch job, update gauge inline
    - Both success and failure branches instrumented independently

key-files:
  created: []
  modified:
    - crates/memcp-core/src/pipeline/gc/worker.rs
    - crates/memcp-core/src/pipeline/gc/dedup.rs
    - crates/memcp-core/src/intelligence/embedding/pipeline.rs
    - crates/memcp-core/src/transport/api/recall.rs
    - crates/memcp-core/src/transport/api/search.rs

key-decisions:
  - "Gauge updates (memcp_memories_total, memcp_memories_pending_embedding) placed inline in embedding worker after each successful job — avoids adding store dependency to the pool poller"
  - "Both empty-result early return AND normal path in search_handler record memcp_search_results_returned to ensure 100% request coverage"
  - "Insert_embedding failure path (store write error) counts as error, not success — instrumented separately from provider embed failure"

patterns-established:
  - "Pattern: Always instrument both success AND failure branches with separate label values"
  - "Pattern: Timer started before provider call, recorded only on success path; error path omits duration"

requirements-completed: [PH-07]

# Metrics
duration: 47min
completed: 2026-03-07
---

# Phase 10 Plan 03: Worker and Handler Prometheus Instrumentation Summary

**GC, dedup, embedding pipeline, recall, and search handlers instrumented with Prometheus counter/histogram/gauge calls completing the full observability picture**

## Performance

- **Duration:** 47 min
- **Started:** 2026-03-07T19:06:03Z
- **Completed:** 2026-03-07T19:53:10Z
- **Tasks:** 1
- **Files modified:** 5

## Accomplishments
- GC worker records run count and pruned count per GC cycle via Prometheus counters
- Dedup worker records merge count on each successful duplicate merge
- Embedding pipeline records per-job success/error counters, duration histogram per tier, and updates memory count gauges after each successful embedding
- Recall and search API handlers record result-count histograms covering all code paths

## Task Commits

1. **Task 1: Instrument workers + API handlers with Prometheus metrics** - `7740d12` (feat)

## Files Created/Modified
- `crates/memcp-core/src/pipeline/gc/worker.rs` - Added memcp_gc_runs_total and memcp_gc_pruned_total counter increments after each GC run
- `crates/memcp-core/src/pipeline/gc/dedup.rs` - Added memcp_dedup_merges_total counter increment on each successful merge
- `crates/memcp-core/src/intelligence/embedding/pipeline.rs` - Added timing, success/error counters, duration histogram, and memory count gauge updates
- `crates/memcp-core/src/transport/api/recall.rs` - Added memcp_recall_memories_returned histogram before building response
- `crates/memcp-core/src/transport/api/search.rs` - Added memcp_search_results_returned histogram on both empty-result early-return and normal path

## Decisions Made
- Gauge updates for memory counts placed inline in embedding worker (not in the pool poller) to keep the poller focused on DB connection metrics and avoid introducing a store reference there
- Empty-result early-return path in search_handler also records the histogram (value 0.0) so every search request contributes a data point regardless of result
- Insert_embedding storage failure is treated as `status=error` (distinct from the embed provider call succeeding) — this gives accurate error accounting even when the provider works but persistence fails

## Deviations from Plan

None - plan executed exactly as written. The `count_live_memories()` and `count_pending_embeddings()` methods already existed on `PostgresMemoryStore` so no new store methods were needed.

## Issues Encountered
- Pre-existing test failure in `benchmark::locomo::dataset::tests::test_load_locomo_dataset_valid` confirmed to be unrelated to these changes (fails without our changes, dataset parse error).

## Next Phase Readiness
- All metric counter/histogram/gauge call sites are now instrumented
- Metrics are live as soon as the daemon starts and the recorder is installed (Plan 10-01)
- Ready for Plan 10-05 (any remaining hardening or alerting rules)

---
*Phase: 10-production-hardening*
*Completed: 2026-03-07*
