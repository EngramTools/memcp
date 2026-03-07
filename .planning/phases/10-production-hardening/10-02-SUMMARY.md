---
phase: 10-production-hardening
plan: 02
subsystem: infra
tags: [metrics, rate-limiting, observability, axum, tower_governor, tower-http, tracing]

# Dependency graph
requires:
  - 10-01 (RateLimitConfig struct, tower_governor dependency, metrics recorder installed)
provides:
  - Metered /v1/* routes (memcp_requests_total + memcp_request_duration_seconds per endpoint)
  - Per-endpoint rate limiting with JSON 429 responses (Retry-After + retry_after_ms body)
  - Enriched /status with pool_active, pool_idle, pending embedding count, model name
  - HTTP request tracing spans (request_id, method, endpoint via TraceLayer)
  - Redacted<T> wrapper for content privacy in logs
affects:
  - 10-03-embedding-metrics (metrics recorder ready, request metrics already emitting)
  - 12-auth (Phase 12 pattern: api::router(&rl).layer(jwt_middleware) documented in api/mod.rs)

# Tech tracking
tech-stack:
  added:
    - tower-http = "0.6" with trace feature (TraceLayer for HTTP request spans)
  patterns:
    - Per-endpoint rate limiting via separate Router sub-merges with GovernorLayer per route
    - metrics_middleware applied to api_routes sub-router only (not /health, /status, /metrics)
    - TraceLayer as outermost layer on full app router for cross-cutting request tracing
    - Redacted<T> Display/Debug wrapper for content privacy in structured logs

key-files:
  created: []
  modified:
    - crates/memcp-core/src/transport/api/mod.rs
    - crates/memcp-core/src/transport/metrics.rs
    - crates/memcp-core/src/transport/health/mod.rs
    - crates/memcp-core/src/logging.rs
    - crates/memcp-core/Cargo.toml

key-decisions:
  - "metrics_middleware applied to api_routes sub-router only — /health and /metrics are excluded from metering to prevent scrape traffic inflating request counters"
  - "Per-endpoint GovernorLayer instances (one per route group) — sharing one layer would prevent per-endpoint RPS config"
  - "TraceLayer as outermost layer on full app — spans cover ALL requests including /health and /metrics, not just /v1/*"
  - "Redacted<T> is Display+Debug only — callers must explicitly wrap values, no implicit redaction that could hide bugs"

# Metrics
duration: 50min
completed: 2026-03-07
---

# Phase 10 Plan 02: Rate Limiting + Metrics Middleware + Status Enrichment Summary

**Per-endpoint rate limiting (GovernorLayer), request metrics middleware, enriched /status with pool breakdown and embedding details, TraceLayer for HTTP request spans, and Redacted<T> for content privacy in logs**

## Performance

- **Duration:** ~50 min
- **Started:** 2026-03-07T17:01:00Z
- **Completed:** 2026-03-07T17:51:45Z
- **Tasks:** 3
- **Files modified:** 5

## Accomplishments

- POST /v1/* requests increment `memcp_requests_total` with endpoint and status labels
- POST /v1/* requests record duration in `memcp_request_duration_seconds` histogram
- /health and /metrics are NOT metered (metrics_middleware applied only to api_routes sub-router)
- Rate limiting active per endpoint: recall/search at 100 RPS, store/annotate/update at 50 RPS, discover/delete at 50 RPS, export at 10 RPS (with 2x burst)
- 429 responses include `Retry-After` header and `{"error":"rate limited","retry_after_ms":N}` JSON body
- `rate_limit.enabled=false` disables all rate limits
- /status now shows `pool_active`, `pool_idle`, `pending` embedding count, and `model` name
- HTTP request logs include `request_id=<uuid>`, `method`, and `endpoint` span via TraceLayer
- `Redacted<T>` wrapper in logging.rs prevents memory content from appearing in INFO-level logs
- Content audit: all existing INFO-level logs confirmed content-free (content appears in debug-level only)

## Task Commits

1. **Task 1: Metrics middleware + per-endpoint rate limiting** - `c5d14f1` (feat)
2. **Task 2: Enrich /status with pool breakdown and embedding details** - `ced71f5` (feat)
3. **Task 3: Redacted<T> + TraceLayer for request spans** - `9b788bd` (feat)

## Files Created/Modified

- `crates/memcp-core/src/transport/api/mod.rs` - build_rate_limit_layer() helper, router(rl) with per-endpoint GovernorLayer instances, covers recall/search/store/annotate/update/discover/delete/export
- `crates/memcp-core/src/transport/metrics.rs` - metrics_middleware function recording counter and histogram per endpoint
- `crates/memcp-core/src/transport/health/mod.rs` - Enriched status_handler with pool_active/pool_idle/pending/model, serve() with metrics middleware and TraceLayer
- `crates/memcp-core/src/logging.rs` - Redacted<T> struct with Display+Debug impls
- `crates/memcp-core/Cargo.toml` - Added tower-http with trace feature

## Decisions Made

- metrics_middleware applied to api_routes sub-router only — ensures /health and /metrics scrape calls don't inflate `memcp_requests_total`
- One GovernorLayer instance per route endpoint — GovernorLayer holds a single quota so sharing would prevent per-endpoint RPS configuration
- TraceLayer applied as outermost app layer — request spans include ALL endpoint types, not just /v1/* ones
- Redacted<T> requires explicit wrapping — implicit redaction could hide bugs where log formatting reveals content accidentally

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] test file `api_test.rs` was calling `api::router()` with no arguments**
- **Found during:** Task 1 verification (cargo test)
- **Issue:** api/mod.rs changed `router()` to `router(rl: &RateLimitConfig)` but api_test.rs already had `api::router(&state.config.rate_limit)` from a prior fix — build cache showed stale error but file was already correct
- **Fix:** Confirmed test was already fixed (stale compiler cache showed false positive)
- **Outcome:** 172/173 tests passing (1 pre-existing failure: benchmark::locomo::dataset::tests::test_load_locomo_dataset_valid)

**2. [Rule 3 - Blocking] Task 1 already partially implemented from Plan 01**
- **Found during:** Initial file reads before execution
- **Issue:** `api/mod.rs` already had full per-endpoint rate limiting code and `metrics.rs` already had `metrics_middleware`. These were committed as part of Plan 01 preparation.
- **Action:** Treated as completed work — verified correctness, no changes needed to these files
- **Outcome:** Plan 02 execution focused on Tasks 2 and 3

---

**Total deviations:** 2 (1 stale build cache false positive, 1 pre-implemented task from Plan 01)
**Impact on plan:** No scope creep. All plan objectives achieved.

## Issues Encountered

- Pre-existing test failure `benchmark::locomo::dataset::tests::test_load_locomo_dataset_valid` — confirmed pre-existing per Plan 01 SUMMARY. Not caused by Plan 02 changes.

## Next Phase Readiness

- Plan 03 (Embedding Metrics): Recorder installed (Plan 01), request metrics emitting (Plan 02) — Plan 03 adds embedding-specific metrics at pipeline callsites
- Plan 04 (GC/Dedup Workers): No dependencies on Plan 02 output
- Phase 12 (Auth): Pattern documented in api/mod.rs docstring: `api::router(&rl_config).layer(jwt_middleware)`

---
*Phase: 10-production-hardening*
*Completed: 2026-03-07*
