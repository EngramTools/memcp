---
phase: 25-reasoning-agent
plan: 08
subsystem: transport
tags: [rust, axum, middleware, byok, security, reasoning, ssrf-mitigation]

# Dependency graph
requires:
  - phase: 25-reasoning-agent
    provides: "ProviderCredentials { api_key, base_url } + ReasoningError + create_reasoning_provider factory (plan 01)"
  - phase: 25-reasoning-agent
    provides: "AgentCallerContext { creds, … } consumed by runner (plan 06)"
  - phase: 24.5-universal-ingestion
    provides: "require_api_key auth layering pattern (plans 02/03) — reasoning middleware composes AFTER it so auth stays outermost"
provides:
  - "require_reasoning_creds axum middleware enforcing BYOK / Pro tenancy policy"
  - "ReasoningCreds state + ReasoningTenancy enum on AppState, loaded from MEMCP_REASONING__<P>_API_KEY at boot"
  - "ProviderCredentials inserted into request extensions for downstream handlers"
  - "Ollama no-auth short-circuit in both tenancies (Reviews HIGH #2)"
  - "D-08 Critical Failure Mode #5 hardening — Pro tier strips caller-supplied x-reasoning-api-key before next.run() and warn!-logs the event (never the key)"
affects: [25.1-openrouter-byok, 26-dreaming-worker, 27-agentic-retrieval]

# Tech tracking
tech-stack:
  added: [http-body-util 0.1 (dev-dep, direct import for in-process Router harness)]
  patterns:
    - "layered middleware composition — reasoning layer applied at merged Router after rate-limit + auth"
    - "tenancy-keyed credential resolution — Pro pulls from env map, BYOK pulls from request headers"
    - "no-auth provider short-circuit before tenancy branching (ollama)"
    - "in-process tower::ServiceExt::oneshot harness for axum middleware tests (no TCP bind)"

key-files:
  created:
    - "crates/memcp-core/src/transport/api/reasoning.rs"
    - "crates/memcp-core/tests/reasoning_byok_boundary.rs (replaces Wave-0 scaffold)"
  modified:
    - "crates/memcp-core/src/transport/health/mod.rs"
    - "crates/memcp-core/src/transport/api/mod.rs"
    - "crates/memcp-core/src/transport/daemon.rs"
    - "crates/memcp-core/src/bin/load_test.rs"
    - "crates/memcp-core/tests/api_test.rs"
    - "crates/memcp-core/tests/ingest_test.rs"
    - "crates/memcp-core/tests/metrics_test.rs"
    - "crates/memcp-core/tests/rate_limit_test.rs"
    - "crates/memcp-core/tests/trust_retrieval_test.rs"
    - "crates/memcp-core/Cargo.toml"

key-decisions:
  - "AppState construction site is transport/daemon.rs:354, NOT transport/server.rs as plan stated — server.rs is the rmcp MemoryService. Used daemon.rs (Rule 3 deviation)."
  - "Middleware applied at merged Router level (end of router()), not per-route, so every /v1/* path observes it while non-reasoning traffic passes through untouched."
  - "Added http-body-util + tower(util) as explicit dev-deps even though transitive via axum 0.8 — direct import clarity in the test harness."
  - "AppState defaults in tests use ReasoningCreds::default() + ReasoningTenancy::Byok so pre-plan-08 test fixtures are byte-identical in behavior (middleware skipped unless x-reasoning-provider header present)."
  - "base_url always None in BYOK path (T-25-01-01 SSRF mitigation inherited from plan 01 — never accept caller-supplied base URLs)."

patterns-established:
  - "Tenancy enum with env-key-derived constructor — ReasoningCreds::from_env().tenancy() returns Pro only when a non-ollama key is present (T-25-08-07 regression guard)"
  - "In-process axum middleware test harness — Router::oneshot() + tower::ServiceExt + http_body_util::BodyExt::collect, zero network I/O"
  - "Pro-tier header scrubbing before next.run() — defense against Critical Failure Mode #5"

requirements-completed: [REAS-04]

# Metrics
duration: ~26min
completed: 2026-04-23
---

# Phase 25 Plan 08: BYOK axum middleware (REAS-04) Summary

**`require_reasoning_creds` middleware + `ReasoningTenancy` enum + `ReasoningCreds` on `AppState` — D-08 hardens Pro to strip caller-supplied `x-reasoning-api-key` before dispatch, Ollama short-circuits to no-auth in both tenancies (Reviews HIGH #2), and all 8 wiring sites (MEDIUM #8) are grep-verifiable.**

## Performance

- **Duration:** ~26 min
- **Started:** 2026-04-23T19:21:00Z (approx — STATE.md shows 19:26:13Z session start)
- **Completed:** 2026-04-23T19:47:22Z
- **Tasks:** 2 (auto-tdd)
- **Files modified:** 10 (2 created, 8 modified)

## Accomplishments

- `require_reasoning_creds` axum middleware in `transport/api/reasoning.rs` enforces closed provider allowlist {kimi, openai, ollama} with 400 on unknown, 401 on BYOK non-ollama missing key, 503 on Pro non-ollama missing env, 200 with `api_key=None` on ollama in either tenancy.
- `ReasoningTenancy::{Pro,Byok}` + `ReasoningCreds { env_keys }` added to `transport/health/mod.rs`; `from_env()` reads `MEMCP_REASONING__{KIMI,OPENAI,OLLAMA}_API_KEY` and `tenancy()` returns Pro only when a non-ollama key is present (T-25-08-07).
- `AppState` extended with `reasoning_creds` + `reasoning_tenancy` fields; 8 struct-literal sites updated (daemon.rs, load_test.rs ×2, 5 test helpers).
- Middleware layered on the merged Router in `transport/api/mod.rs::router` on both rate-limit-enabled and disabled branches. Signature extended to 4 args; all 7 call sites updated (daemon via health::serve, 1 load-test bin, 5 integration tests).
- 8 BYOK boundary tests green using in-process `Router::oneshot` (no TCP bind). Covers D-08 strip, BYOK requires, unknown-provider 400, Pro 503, passthrough, BYOK happy path, plus 2 HIGH #2 ollama regressions (BYOK-no-key + Pro-no-env).
- Never logs the `api_key` value — only event-name + provider tracing fields (T-25-08-02).

## Task Commits

1. **Task 1: `require_reasoning_creds` middleware + `ReasoningTenancy/Creds` on AppState** — `10de3c7` (feat)
2. **Task 2: 8 BYOK boundary security tests green** — `cb9ced4` (test)

**Plan metadata:** (to be added — commit of this SUMMARY + STATE/ROADMAP updates)

## Files Created/Modified

- `crates/memcp-core/src/transport/api/reasoning.rs` **(created)** — 163 LOC. `require_reasoning_creds` middleware + `ReasoningMwState` + `ALLOWED_PROVIDERS` + ollama short-circuit + Pro-strip path + BYOK-require path.
- `crates/memcp-core/tests/reasoning_byok_boundary.rs` **(replaced Wave-0 scaffold)** — 217 LOC. 8 `#[tokio::test]`s via `Router::oneshot`.
- `crates/memcp-core/src/transport/health/mod.rs` — added `ReasoningTenancy` enum + `ReasoningCreds` struct (+ `from_env` + `tenancy`); 2 new fields on `AppState`; `serve()` threads them into `api::router`.
- `crates/memcp-core/src/transport/api/mod.rs` — `pub mod reasoning;` registration; `router()` signature `(rl, auth_state, reasoning_tenancy, reasoning_creds)`; middleware layered on both branches.
- `crates/memcp-core/src/transport/daemon.rs` — boot reads `ReasoningCreds::from_env()` + `.tenancy()`, passes to AppState at line 354.
- `crates/memcp-core/src/bin/load_test.rs` — 2 AppState literals + 1 router call updated.
- `crates/memcp-core/tests/{api,ingest,metrics,rate_limit,trust_retrieval}_test.rs` — AppState literals + router calls updated (defaults to Byok + empty creds so existing tests unaffected).
- `crates/memcp-core/Cargo.toml` — `http-body-util = "0.1"` + `tower = { version = "0.5", features = ["util"] }` added to `[dev-dependencies]` for direct-import in the test harness.

## Decisions Made

- **AppState site = daemon.rs, not server.rs** — plan referenced `transport/server.rs` but that file is the rmcp MCP `MemoryService`. Real AppState construction is `transport/daemon.rs:354`. Used daemon.rs and documented.
- **Router-level middleware layer** — applied via `.layer(from_fn_with_state)` at the merged Router in `router()` (both enabled + disabled branches), so every /v1/* endpoint observes it. Non-reasoning requests pass through untouched (no `x-reasoning-provider` header → early-return).
- **`router()` signature extended to 4 args** — caller threading the 2 new state fields is the simplest way to avoid re-deriving ReasoningCreds inside `router()` from AppState (AppState is `Router<AppState>` state, but the `from_fn_with_state` needs its own cloneable state, not AppState).
- **Test defaults = `Byok` + empty `env_keys`** — keeps all pre-plan-08 tests byte-identical since they don't set `x-reasoning-provider` and the middleware short-circuits to pass-through.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] AppState construction site differs from plan**
- **Found during:** Task 1 (AppState extension + boot wiring)
- **Issue:** Plan's `Wiring Checklist` row #3 pointed to `crates/memcp-core/src/transport/server.rs` for daemon AppState construction, but `server.rs` is the rmcp MCP `MemoryService` handler — it has no `AppState {` literal. The real AppState construction is in `transport/daemon.rs:354`.
- **Fix:** Wired `ReasoningCreds::from_env()` + `tenancy()` in `daemon.rs` instead.
- **Files modified:** `crates/memcp-core/src/transport/daemon.rs`
- **Verification:** `grep -c 'reasoning_creds\|reasoning_tenancy' crates/memcp-core/src/transport/daemon.rs` returns 4.
- **Committed in:** 10de3c7

**2. [Rule 2 - Missing Critical] 7 additional AppState struct-literal sites not in plan**
- **Found during:** Task 1 (build phase after adding fields to AppState)
- **Issue:** Plan mentioned 24.75-04's precedent of 8 AppState literal sites but didn't enumerate them. Found 8 total via `grep -rn "AppState {"`: daemon.rs (1), load_test.rs (2), api_test.rs (1), ingest_test.rs (1), metrics_test.rs (1), rate_limit_test.rs (2), trust_retrieval_test.rs (1). All 8 needed `reasoning_creds` + `reasoning_tenancy` fields or the build breaks (E0063 missing-field).
- **Fix:** Updated all 8 sites with safe defaults (`ReasoningCreds::default()` + `ReasoningTenancy::Byok`) so test behavior is unchanged.
- **Files modified:** 1 bin + 5 test files.
- **Verification:** `cargo build -p memcp-core` + all affected test binaries compile; no test regressions.
- **Committed in:** 10de3c7

**3. [Rule 3 - Blocking] `api::router()` signature extended — 7 call sites updated**
- **Found during:** Task 1 (after adding 2 new params to `router()`)
- **Issue:** 6 tests + 1 load-test bin called `api::router(&rl, auth_state)`. Plan didn't call out these call sites.
- **Fix:** Extended all calls to pass `state.reasoning_tenancy` + `state.reasoning_creds.clone()`.
- **Files modified:** `health/mod.rs::serve`, `load_test.rs:434`, and the same 5 integration tests.
- **Verification:** `cargo build -p memcp-core --tests` clean.
- **Committed in:** 10de3c7

**4. [Rule 2 - Missing Critical] Direct `http-body-util` + `tower(util)` dev-deps**
- **Found during:** Task 2 (test harness write)
- **Issue:** Plan listed them as "most axum 0.7 projects have it transitively" — this crate is axum 0.8 and they ARE transitive, but adding `use http_body_util::BodyExt` and `use tower::util::ServiceExt` to a test file requires them as direct deps.
- **Fix:** Added to `[dev-dependencies]`: `tower = { version = "0.5", features = ["util"] }` + `http-body-util = "0.1"`.
- **Files modified:** `crates/memcp-core/Cargo.toml`.
- **Verification:** `cargo test -p memcp-core --test reasoning_byok_boundary`: 8 passed.
- **Committed in:** cb9ced4

**5. [Rule 1 - Bug] `doc_markdown` lint on `api_key` in middleware docstring**
- **Found during:** Post-Task 1 clippy sweep on my files
- **Issue:** Workspace-pedantic `doc_markdown` lint flagged `api_key` (unbacktick'd) in the `require_reasoning_creds` docstring.
- **Fix:** Wrapped in backticks: ``NEVER logs the `api_key` value``.
- **Files modified:** `crates/memcp-core/src/transport/api/reasoning.rs`.
- **Verification:** `cargo clippy -p memcp-core --lib 2>&1 | grep reasoning.rs` returns no warnings on lines I own.
- **Committed in:** cb9ced4

---

**Total deviations:** 5 auto-fixed (2 Rule 3 blocking, 2 Rule 2 missing-critical, 1 Rule 1 bug)
**Impact on plan:** All deviations necessary for the build to compile and the plan's goals to be testable. No scope creep — everything is strictly in service of the Wiring Checklist (MEDIUM #8) or Rust compile-time invariants. The AppState-site correction is the one worth highlighting for future planners — mechanical `grep "AppState {"` before editing is now doubly validated.

## Issues Encountered

- Pre-existing `cargo clippy --lib` error in `transport/api/memory_span.rs:243` (unwrap on Result) surfaced during clippy sweep. NOT my file, NOT introduced by this plan — out of scope per executor scope boundary rule. Logged for a future sweep follow-up (already tracked in STATE.md Next Steps item 3: "workspace-wide clippy sweep follow-up").

## Reviews Revisions Closed

- **HIGH #2 (Ollama must not require API key):** CLOSED — middleware short-circuits when `provider == "ollama"` before any tenancy branching. 2 dedicated tests guard both tenancies (`test_byok_ollama_no_api_key_required`, `test_pro_ollama_no_env_key_succeeds`) and assert `body["api_key"].is_null()` so a future regression would need to actively populate a credential rather than slip in silently.
- **MEDIUM #8 (transport wiring completeness):** CLOSED — all 6 grep targets from the plan's Wiring Checklist return matches:
  | # | Check | Result |
  |-|-|-|
  | 1 | `pub struct AppState` in health/mod.rs | 1 |
  | 2 | AppState fields (`ReasoningTenancy`, `ReasoningCreds`) in health/mod.rs | 1+1 |
  | 3 | `reasoning_creds\|reasoning_tenancy` in daemon.rs (plan said server.rs — corrected) | 4 |
  | 4 | `require_reasoning_creds` in api/mod.rs router layer | 3 |
  | 5 | `pub async fn require_reasoning_creds` in api/reasoning.rs | 1 |
  | 6 | `pub mod reasoning` in api/mod.rs | 1 |

## Threat Model Confirmation

| Threat ID | Mitigation test |
|-|-|
| T-25-08-01 (caller injects rogue key on Pro) | `test_pro_tier_strips_caller_api_key_header` — Pro sees SERVER_ENV_KEY not ROGUE_KEY |
| T-25-08-02 (api_key logged) | `grep -E 'warn!.*api_key[^_]\|key.*=.*%.*api_key' reasoning.rs` returns 0 |
| T-25-08-04 (provider case-fold bypass) | `.trim().to_ascii_lowercase()` before allowlist check |
| T-25-08-06 (Ollama blocked by BYOK key requirement) | 2 HIGH #2 tests |
| T-25-08-07 (tenancy flips to Pro on ollama-only env) | `tenancy()` uses `.any(\|p\| p != "ollama")` — covered by the `test_pro_ollama_no_env_key_succeeds` setup which requires a non-ollama key in the env map to reach Pro |

## User Setup Required

None — this is a server-side middleware plan. Operators configure Pro tier via existing `MEMCP_REASONING__{KIMI,OPENAI,OLLAMA}_API_KEY` env vars (already referenced in plan 01's `ProviderCredentials::from_env`). BYOK callers supply `x-reasoning-provider` + `x-reasoning-api-key` headers on each request.

## Next Phase Readiness

- Phase 25 COMPLETE: all 9 plans (00–08) shipped. REAS-01/04/06/07/08/09/10 done; REAS-02/03/05 delivered as part of the adapter plans (02/03/04); REAS-04 transport rail done.
- Phase 25.1 OpenRouter-via-BYOK can compose directly against this middleware — the header extraction path + `ProviderCredentials` extension pattern are the hand-off; OpenRouter just registers a new entry in `ALLOWED_PROVIDERS` and a factory arm in `create_reasoning_provider`.
- Phase 26 Dreaming Worker can assume `ctx.creds` carries a valid `ProviderCredentials` because every reasoning call now flows through this middleware at the HTTP boundary.
- Phase 27 Agentic Retrieval inherits the same boundary — the retrieval profile's BYOK story is already wired.

## Self-Check: PASSED

**File existence:**
- FOUND: crates/memcp-core/src/transport/api/reasoning.rs
- FOUND: crates/memcp-core/tests/reasoning_byok_boundary.rs

**Commit existence:**
- FOUND: 10de3c7 (Task 1 feat commit)
- FOUND: cb9ced4 (Task 2 test commit)

**Test results:**
- FOUND: 8 passed / 0 failed / 0 ignored in reasoning_byok_boundary
- FOUND: 33 reasoning tests green across loop/termination/budget/tool_dispatch/tools/trait/kimi/ollama/openai (no regressions)

---
*Phase: 25-reasoning-agent*
*Completed: 2026-04-23*
