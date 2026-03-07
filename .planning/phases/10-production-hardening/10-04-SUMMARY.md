---
phase: 10-production-hardening
plan: "04"
subsystem: observability
tags: [rate-limiting, metrics, workers, prometheus, production-hardening]
dependency_graph:
  requires: [10-01, 10-02]
  provides: [complete-rate-limit-coverage, worker-prometheus-counters, discover-histogram]
  affects: [api-router, config, enrichment-worker, promotion-worker, curation-worker, temporal-worker, discover-handler]
tech_stack:
  added: [governor = "0.10", tower = "0.5"]
  patterns: [per-endpoint-governor-layers, metrics-counter-at-sweep-completion, histogram-at-handler-return]
key_files:
  created: []
  modified:
    - crates/memcp-core/src/config.rs
    - crates/memcp-core/Cargo.toml
    - crates/memcp-core/src/pipeline/enrichment/worker.rs
    - crates/memcp-core/src/pipeline/promotion/worker.rs
    - crates/memcp-core/src/pipeline/curation/worker.rs
    - crates/memcp-core/src/pipeline/temporal/mod.rs
    - crates/memcp-core/src/transport/api/discover.rs
    - crates/memcp-core/tests/api_test.rs
decisions:
  - "Added governor = 0.10 and tower = 0.5 as direct deps since tower_governor re-exports governor internally at 0.10.4 and explicit type annotation requires direct access"
  - "Curation counters only increment on non-dry-run path — dry-run produces no side effects including metrics"
  - "Enrichment and promotion counters placed after for-loop (per-sweep, not per-memory for promotion)"
metrics:
  duration_minutes: 45
  completed_date: "2026-03-07"
  tasks_completed: 2
  tasks_total: 2
  files_modified: 8
---

# Phase 10 Plan 04: Gap Closure — Rate Limits and Worker Metrics Summary

Close gaps in Plans 10-02 and 10-03 caused by codebase evolution: added `discover_rps`, `delete_rps`, `export_rps` to `RateLimitConfig` to fix a compile-time gap where `api/mod.rs` referenced these fields before they existed in the struct, and instrumented 4 background workers (enrichment, promotion, curation, temporal) and the discover handler with Prometheus counters and histograms.

## What Was Built

### Task 1: Extend RateLimitConfig + Add New Metric Descriptions

**Gap found:** The 10-02 commit had already updated `api/mod.rs` to use `rl.discover_rps`, `rl.delete_rps`, `rl.export_rps` for per-endpoint rate limiting, but `config.rs` was missing those three fields from `RateLimitConfig`. This caused a compile error in the main branch.

**Fix:**
- Added `discover_rps: u32` (default 50), `delete_rps: u32` (default 50), `export_rps: u32` (default 10) to `RateLimitConfig` in `config.rs`
- Added corresponding `default_*` functions and `Default` impl entries
- Added `governor = "0.10"` and `tower = "0.5"` as direct dependencies (required for explicit type annotation on `build_rate_limit_layer`)

**Result:** All 8 `/v1/*` endpoint groups are now fully rate-limited with per-endpoint config: recall (100 rps), search (100 rps), store (50 rps), annotate (50 rps), update (50 rps), discover (50 rps), delete (50 rps), export (10 rps).

### Task 2: Worker and Handler Metrics

Instrumented all 4 background workers and the discover HTTP handler:

| Worker/Handler | Metric(s) Added |
|-|-|
| `enrichment/worker.rs` | `memcp_enrichment_sweeps_total` (per sweep) + `memcp_enrichment_memories_total` (count when >0) |
| `promotion/worker.rs` | `memcp_promotion_sweeps_total` (per sweep) + `memcp_promotion_promoted_total` (count when >0) |
| `curation/worker.rs` | `memcp_curation_runs_total` + `memcp_curation_merged_total` + `memcp_curation_flagged_total` (non-dry-run only) |
| `temporal/mod.rs` | `memcp_temporal_extractions_total` (per successful `update_event_time` DB write) |
| `api/discover.rs` | `memcp_discover_results_returned` histogram (results count per call) |

Also fixed `tests/api_test.rs` to pass `&state.config.rate_limit` to `api::router()` (signature updated in 10-02 but test wasn't updated).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] 10-02 commit referenced non-existent config fields**
- **Found during:** Task 1 initial cargo check
- **Issue:** `api/mod.rs` used `rl.discover_rps`, `rl.delete_rps`, `rl.export_rps` in the 10-02 commit, but `config.rs` never added those fields — compile broken
- **Fix:** Added the three missing fields to `RateLimitConfig` with serde defaults
- **Files modified:** `crates/memcp-core/src/config.rs`
- **Commit:** 70b5eda

**2. [Rule 1 - Bug] api_test.rs called router() with no args after signature change**
- **Found during:** Task 2 cargo test
- **Issue:** `tests/api_test.rs` still called `api::router()` without args; function now requires `&RateLimitConfig`
- **Fix:** Updated call to `api::router(&state.config.rate_limit)`
- **Files modified:** `crates/memcp-core/tests/api_test.rs`
- **Commit:** 0030667

**3. [Dependency addition] governor and tower as direct deps**
- **Found during:** Task 1 — `build_rate_limit_layer` return type uses `::governor::middleware::StateInformationMiddleware`
- **Issue:** `governor` was only a transitive dep of `tower_governor`; Rust requires direct dependency for use in type annotations
- **Fix:** Added `governor = "0.10"` (matching tower_governor's internal version 0.10.4) and `tower = "0.5"` as direct deps
- **Files modified:** `crates/memcp-core/Cargo.toml`
- **Commit:** 70b5eda

## Verification Results

- `cargo build` passes
- 71 unit tests pass (0 failures)
- Integration tests (api_test, gc_dedup_test) require `DATABASE_URL` — pre-existing requirement unrelated to these changes
- 1 benchmark test (`test_load_locomo_dataset_valid`) failing due to pre-existing dataset parsing issue (unrelated to this plan)

## Self-Check: PASSED

Files verified:
- `crates/memcp-core/src/config.rs` — contains `discover_rps`, `delete_rps`, `export_rps` fields
- `crates/memcp-core/src/pipeline/enrichment/worker.rs` — contains `memcp_enrichment_sweeps_total` counter
- `crates/memcp-core/src/pipeline/promotion/worker.rs` — contains `memcp_promotion_sweeps_total` counter
- `crates/memcp-core/src/pipeline/curation/worker.rs` — contains `memcp_curation_runs_total` counter
- `crates/memcp-core/src/pipeline/temporal/mod.rs` — contains `memcp_temporal_extractions_total` counter
- `crates/memcp-core/src/transport/api/discover.rs` — contains `memcp_discover_results_returned` histogram

Commits verified:
- 70b5eda — feat(10-04): extend RateLimitConfig with discover_rps, delete_rps, export_rps
- 0030667 — feat(10-04): instrument enrichment, promotion, curation, temporal workers + discover
