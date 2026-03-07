---
phase: 10-production-hardening
plan: "05"
subsystem: observability
tags: [integration-tests, prometheus, rate-limiting, metrics, production-hardening]
dependency_graph:
  requires: [10-01, 10-02, 10-03, 10-04]
  provides: [metrics-integration-tests, rate-limit-integration-tests]
  affects: [tests/metrics_test.rs, tests/rate_limit_test.rs, transport/api/mod.rs]
tech_stack:
  added: []
  patterns: [OnceLock-recorder-guard, concurrent-tokio-burst, burst-size-1-rate-limit-testing]
key_files:
  created:
    - crates/memcp-core/tests/metrics_test.rs
    - crates/memcp-core/tests/rate_limit_test.rs
  modified:
    - crates/memcp-core/src/transport/api/mod.rs
decisions:
  - "OnceLock used to install Prometheus recorder exactly once across all tests in the process — subsequent tests get the same handle without panic"
  - "Rate limit tests use concurrent tokio::spawn burst with burst_size=1 rather than sequential requests to guarantee 429 without timing sensitivity"
  - "panic::catch_unwind in rate_limit_test recorder guard prevents panic if metrics_test runs first in the same process and has already installed the global recorder"
  - "Fixed pre-existing bug: /v1/memories/:id → /v1/memories/{id} (axum v0.7 path param syntax), was panicking on router construction"
metrics:
  duration_minutes: 16
  completed_date: "2026-03-07"
  tasks_completed: 2
  tasks_total: 2
  files_modified: 3
---

# Phase 10 Plan 05: Integration Tests for Metrics + Rate Limiting Summary

11 new integration tests across 2 test files that codify Phase 10 observability contracts: Prometheus /metrics endpoint returns valid scrape text with declared metric names, metrics middleware correctly instruments /v1/* routes (but not /health or /metrics themselves), /status exposes pool breakdown fields, rate limiting produces correct 429 responses with Retry-After header and JSON body, and disabled rate limiting allows all requests through.

## What Was Built

### Task 1: Metrics Endpoint + Middleware Integration Tests (`metrics_test.rs`)

6 integration tests following the established `api_test.rs` pattern:

| Test | Verifies |
|-|-|
| `test_metrics_endpoint_returns_prometheus_text` | GET /metrics returns 200 with `# HELP`/`# TYPE` headers and `memcp_requests_total` + `memcp_request_duration_seconds` after a /v1/ request |
| `test_metrics_endpoint_not_metered` | `/metrics` is NOT counted in `memcp_requests_total` (outside /v1/* sub-router) |
| `test_health_endpoint_not_metered` | `/health` is NOT counted in `memcp_requests_total` |
| `test_api_request_increments_counter` | POST /v1/store triggers `memcp_requests_total{endpoint="/v1/store"}` counter |
| `test_api_request_records_duration` | POST /v1/store triggers `memcp_request_duration_seconds_bucket` histogram entries |
| `test_status_shows_pool_details` | GET /status returns `components.db.pool_active` and `components.db.pool_idle` fields |

Key implementation detail: Prometheus recorder installation is guarded by `OnceLock<PrometheusHandle>` — tests share the same global recorder (safe because they only assert presence, not exact values).

### Task 2: Rate Limiting Integration Tests (`rate_limit_test.rs`)

5 integration tests with extremely low limits (rps=1, burst=1) to guarantee 429s without timing sensitivity:

| Test | Verifies |
|-|-|
| `test_rate_limit_returns_429` | 10 concurrent requests with burst_size=1 → at least one 429 |
| `test_rate_limit_429_has_retry_after_header` | 429 response has `Retry-After` header |
| `test_rate_limit_429_has_json_body` | 429 response has `{"error":"rate limited","retry_after_ms":N}` JSON body |
| `test_rate_limit_disabled_allows_all` | `rate_limit.enabled=false` → no 429s even under burst |
| `test_rate_limit_health_not_limited` | 20 concurrent GET /health → never 429 (health outside /v1/) |

Rate limit tests use `tokio::spawn` for concurrent bursting rather than sequential requests — this ensures the token bucket sees simultaneous arrivals rather than benefiting from async scheduling gaps.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] axum v0.7 path param syntax: `:id` → `{id}` in api/mod.rs**
- **Found during:** Task 1 initial test run — all tests panicked on router construction
- **Issue:** `api/mod.rs` used old axum v0.6 colon syntax (`.route("/v1/memories/:id", ...)`) for the DELETE route. axum v0.7 requires curly-brace syntax (`{id}`) and panics at route registration with pre-existing path segments starting with `:`
- **Fix:** Updated both the rate-limited path (line 145) and the disabled path (line 114) in `api::router()` to use `/v1/memories/{id}`
- **Files modified:** `crates/memcp-core/src/transport/api/mod.rs`
- **Commit:** cd064ba

**2. [Rule 2 - Missing functionality] OnceLock recorder guard in rate_limit_test.rs**
- **Found during:** Task 2 design — recognized that metrics_test.rs and rate_limit_test.rs may run in the same test process
- **Issue:** Installing a global Prometheus recorder twice panics. If metrics_test runs before rate_limit_test in the same process, the OnceLock in metrics_test would already hold the handle, but a fresh installation attempt would panic
- **Fix:** `get_or_install_recorder()` in rate_limit_test.rs wraps `install_prometheus_recorder()` in `panic::catch_unwind` and falls back to a local non-global recorder. This is safe because rate_limit tests don't need /metrics output
- **Files modified:** `crates/memcp-core/tests/rate_limit_test.rs`

## Verification Results

- `cargo test --test metrics_test` — 6 passed, 0 failed
- `cargo test --test rate_limit_test` — 5 passed, 0 failed
- `cargo test --test api_test` — 15 passed, 0 failed (pre-existing tests unaffected)
- Combined run of all three test binaries: 26 total, 0 failed

## Self-Check: PASSED

Files verified:
- `crates/memcp-core/tests/metrics_test.rs` — exists, 183 lines, 6 test functions
- `crates/memcp-core/tests/rate_limit_test.rs` — exists, 316 lines, 5 test functions
- `crates/memcp-core/src/transport/api/mod.rs` — both delete routes use `{id}` syntax

Commits verified:
- cd064ba — test(10-05): add metrics endpoint + middleware integration tests
- 97b450d — test(10-05): add rate limiting integration tests
