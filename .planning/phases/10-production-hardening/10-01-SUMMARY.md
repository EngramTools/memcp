---
phase: 10-production-hardening
plan: 01
subsystem: infra
tags: [prometheus, metrics, observability, axum, tower_governor, sqlx, postgres]

# Dependency graph
requires: []
provides:
  - Prometheus metrics recorder installed globally at daemon startup
  - GET /metrics endpoint returning Prometheus scrape text (13 metrics described)
  - RateLimitConfig and ObservabilityConfig structs in config.rs
  - pool metrics background poller (gauges every 10s)
  - max_db_connections from ResourceCapsConfig wired into PgPoolOptions
  - count_pending_embeddings() method on PostgresMemoryStore
affects:
  - 10-02-rate-limiting (uses RateLimitConfig + tower_governor dependency)
  - 10-03-embedding-metrics (recorder already installed, can use metrics macros)
  - 14-gap-closure-integration-tests (AppState now requires metrics_handle)

# Tech tracking
tech-stack:
  added:
    - metrics = "0.24" (metrics facade crate)
    - metrics-exporter-prometheus = "0.18" (Prometheus backend + PrometheusHandle)
    - tower_governor = "0.8" with axum feature (rate limiting middleware, for Plan 02)
  patterns:
    - Metrics recorder installed once at daemon startup before any metrics macros run
    - PrometheusHandle stored in AppState for handler access (no global state in handlers)
    - Test isolation via PrometheusBuilder::new().build_recorder().handle() (non-global recorder)
    - Pool constructor pattern: new_with_schema_internal() shared by public constructors + new_with_pool_config()

key-files:
  created:
    - crates/memcp-core/src/transport/metrics.rs
  modified:
    - crates/memcp-core/Cargo.toml
    - crates/memcp-core/src/config.rs
    - crates/memcp-core/src/transport/mod.rs
    - crates/memcp-core/src/transport/health/mod.rs
    - crates/memcp-core/src/transport/daemon.rs
    - crates/memcp-core/src/storage/store/postgres.rs
    - crates/memcp-core/tests/api_test.rs

key-decisions:
  - "Used new_with_pool_config() constructor instead of changing new() signature to avoid cascading caller updates across 6+ callsites"
  - "PrometheusHandle stored in AppState (not accessed globally) to avoid hidden shared state in handlers"
  - "Test AppState uses non-global build_recorder().handle() for recorder isolation across concurrent tests"
  - "Pool metrics poller only spawned when health server is enabled (consistent with daemon-only pattern)"
  - "Recorder installed only in daemon path (health.enabled block) — MCP serve and CLI paths don't install Prometheus"

patterns-established:
  - "metrics.rs module pattern: all prometheus setup in one file (install, describe, poll, handler)"
  - "Daemon config wiring: observability.pool_poll_interval_secs controls background poller cadence"

requirements-completed: [PH-01, PH-02, PH-04]

# Metrics
duration: 35min
completed: 2026-03-07
---

# Phase 10 Plan 01: Prometheus Metrics Foundation Summary

**Prometheus recorder installed at daemon startup with /metrics endpoint, 13 metric descriptions, pool connection gauges, and max_db_connections config wired into PgPoolOptions**

## Performance

- **Duration:** 35 min
- **Started:** 2026-03-07T07:10:00Z
- **Completed:** 2026-03-07T07:45:54Z
- **Tasks:** 2
- **Files modified:** 7 (+ 1 created)

## Accomplishments

- GET /metrics endpoint returns Prometheus scrape text via PrometheusHandle stored in AppState
- All 13 metrics declared: requests, duration, memories_total, pending_embedding, embedding_jobs, embedding_duration, recall/search results, db_pool_connections, pool_acquire_duration, gc_runs, gc_pruned, dedup_merges
- Pool connection gauges update every 10s with active/idle labels
- max_db_connections from ResourceCapsConfig replaces hardcoded 10 in PgPoolOptions (daemon path)
- RateLimitConfig and ObservabilityConfig structs ready for Plans 02 and 03
- tower_governor dependency added (Plan 02 can use without Cargo.toml changes)

## Task Commits

1. **Task 1: Add dependencies + config structs + metrics module** - `d3697e8` (feat)
2. **Task 2: Wire metrics into AppState, /metrics route, pool config, daemon startup** - `02de2ac` (feat)

## Files Created/Modified

- `crates/memcp-core/src/transport/metrics.rs` - Prometheus recorder install, metric descriptions, pool poller, /metrics handler
- `crates/memcp-core/Cargo.toml` - Added metrics, metrics-exporter-prometheus, tower_governor dependencies
- `crates/memcp-core/src/config.rs` - Added RateLimitConfig and ObservabilityConfig structs with serde defaults, registered in Config
- `crates/memcp-core/src/transport/mod.rs` - Registered pub mod metrics
- `crates/memcp-core/src/transport/health/mod.rs` - Added metrics_handle: PrometheusHandle to AppState, /metrics route in serve()
- `crates/memcp-core/src/transport/daemon.rs` - Install recorder, describe metrics, spawn pool poller, wire metrics_handle into AppState, use new_with_pool_config
- `crates/memcp-core/src/storage/store/postgres.rs` - new_with_pool_config(), new_with_schema_internal(), count_pending_embeddings()
- `crates/memcp-core/tests/api_test.rs` - Added non-global PrometheusRecorder for test AppState

## Decisions Made

- Used `new_with_pool_config()` instead of changing `new()` signature — 6+ callers of `new()` across main.rs and cli.rs would all need updating; new constructor is cleaner
- PrometheusHandle goes in AppState (not thread-local or global) — handlers receive it via axum State extractor, consistent with other AppState fields
- Non-global `PrometheusBuilder::new().build_recorder().handle()` for tests — avoids global recorder conflicts between concurrent test cases
- Prometheus recorder installed only when `config.health.enabled = true` — daemon-only feature, MCP serve and CLI don't get a metrics endpoint

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Added test metrics_handle for api_test.rs**
- **Found during:** Task 2 verification (cargo test)
- **Issue:** `AppState` gained `metrics_handle` field but `api_test.rs` constructs `AppState` directly — missing field compile error
- **Fix:** Added `metrics-exporter-prometheus` to dev-dependencies; created non-global PrometheusRecorder for test AppState using `PrometheusBuilder::new().build_recorder().handle()`
- **Files modified:** crates/memcp-core/tests/api_test.rs, crates/memcp-core/Cargo.toml
- **Verification:** cargo test passes (172/172 non-benchmark tests pass)
- **Committed in:** 02de2ac (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (Rule 2 - missing critical for test compilation)
**Impact on plan:** Fix necessary for test suite to compile. No scope creep.

## Issues Encountered

- Pre-existing test failure `benchmark::locomo::dataset::tests::test_load_locomo_dataset_valid` — confirmed pre-existing by stashing and testing without changes. Not caused by this plan.

## Next Phase Readiness

- Plan 02 (Rate Limiting): `RateLimitConfig` struct available, `tower_governor` dependency in Cargo.toml, no Cargo changes needed
- Plan 03 (Embedding Metrics): Prometheus recorder installed, all 13 metrics described — Plans 03 just needs to emit the macros at the right callsites
- `/metrics` endpoint ready for Prometheus scraping at `localhost:9090/metrics`

---
*Phase: 10-production-hardening*
*Completed: 2026-03-07*
