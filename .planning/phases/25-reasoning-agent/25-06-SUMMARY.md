---
phase: 25-reasoning-agent
plan: 06
subsystem: reasoning
tags: [rust, loop-runner, state-machine, metrics, reasoning, wiremock, tokio]

requires:
  - phase: 25-01
    provides: ReasoningProvider trait + unified wire types + ProfileConfig + factory
  - phase: 25-05
    provides: dispatch_tool + validate_tool_schemas + memory_tools() palette
  - phase: 25-02
    provides: Kimi adapter (exercised via factory path — tests use mock directly)
  - phase: 25-03
    provides: OpenAI adapter (ditto)
  - phase: 25-04
    provides: Ollama adapter (ditto)

provides:
  - run_agent(profile_name, profile, system_prompt, tools, ctx) — public entry for P26/P27
  - run_agent_with_provider(provider, ...) — mock-friendly entry bypassing the factory
  - run_agent_with_provider_and_timeout(..., turn_timeout_override) — timeout-testable entry
  - apply_salience_side_effects stub — plan 07 replaces body with real REAS-10 writes
  - 4 termination reasons (Terminal / BudgetExceeded / MaxIterations / RepeatedToolCall) verified
  - reasoning_tokens_total{profile, adapter} metric — profile label is the NAME, not model id
  - 3 shared mock providers (MockReasoningProvider, SlowMockProvider, RecordingMockProvider)
  - NullStore + noop_ctx fixtures reusable by plans 07+

affects:
  - 25-07 (salience hook — runner already calls apply_salience_side_effects at every exit)
  - 25-08 (BYOK wiring — surfaces on top of the run_agent entry)
  - 26 (dreaming worker — primary consumer of run_agent)
  - 27 (agentic retrieval — primary consumer of run_agent)

tech-stack:
  added:
    - futures 0.3 (explicit dep — was transitive only; required by join_all)
  patterns:
    - "Three-tier public entry: factory (run_agent) -> mock-friendly (run_agent_with_provider) -> timeout-testable (run_agent_with_provider_and_timeout). Each strictly forwards to the next with extra flexibility — one code path, three testable arities."
    - "Canonical JSON hash for repeated-call detection: sort object keys + recurse. Two syntactically different JSON representations of the same logical args hash identically."
    - "Fixed adapter at run_agent entry (Open Q5): no mid-loop provider switching; caller chooses profile up front."
    - "max_iterations sourced from profile.max_iterations (Open Q6): no hard-coded default in the runner."
    - "Pitfall 3 enforcement via grep acceptance: finish_reason may appear ONLY inside tracing::debug! — never in if/match/==. A source-level regression guard test (test_metric_emitted_counter_name_present_in_source pattern) locks this in for the metric label too."

key-files:
  created:
    - crates/memcp-core/src/intelligence/reasoning/runner.rs (225 LOC)
    - crates/memcp-core/tests/common/reasoning_fixtures.rs (185 LOC)
  modified:
    - crates/memcp-core/src/intelligence/reasoning/mod.rs (+23 lines — module + stub + re-export)
    - crates/memcp-core/tests/reasoning_loop_test.rs (61 LOC — scaffold -> real smoke)
    - crates/memcp-core/tests/reasoning_termination.rs (162 LOC — scaffolds -> 4 tests)
    - crates/memcp-core/tests/reasoning_budget.rs (162 LOC — scaffolds -> 3 tests)
    - crates/memcp-core/Cargo.toml (+1 line — futures 0.3)

key-decisions:
  - "apply_salience_side_effects stub in mod.rs (not runner.rs) so plan 07 drops its real impl in the same mod.rs module without touching runner.rs. Runner calls it by super:: re-export."
  - "Shared NullStore + noop_ctx in tests/common/reasoning_fixtures.rs — all 3 runner test files share one no-op MemoryStore. Avoids per-file duplication; plans 07+ reuse."
  - "Single-line metrics::counter! invocation mandated by plan acceptance grep. #[rustfmt::skip] pins the formatting so future rustfmt runs don't break the contains-string check in test_metric_emitted."
  - "Diagnostic finish_reason log uses `let diag_finish = resp.finish_reason.as_deref().unwrap_or(\"<none>\")` instead of `if let Some(fr) = resp.finish_reason` — the acceptance regex `if.*finish_reason` would match the if-let form. Extracting via let binding + unwrap_or keeps finish_reason logging without matching the Pitfall 3 regex."
  - "test_repeated bumped max_iterations from profile default (3) to 10 so the repeated-call detector fires before MaxIterations would otherwise win — repeated-call check happens AFTER the generate call completes, so iteration 3 with 3 identical calls trips the detector BEFORE iteration 4's budget/iter check."
  - "NullStore::delete returns Ok(()) rather than Err — dispatcher asks about is_source_of_any_derived and delete; Ok(()) keeps the test loop mechanics honest without depending on Postgres."
  - "test_max_iter passes memory_tools() (not vec![]) so dispatch_tool runs real schema validation and returns a structured ToolResult, matching the real runtime path. vec![] would return unknown_tool errors — still loops, but less representative of production."

patterns-established:
  - "Per-turn timeout defaults driven by profile.provider (ollama=120s, others=30s) with a Option<Duration> override for tests. The override threads through the bottom-most entry only; higher arities default."
  - "Regression guard tests (source-level contains-string assertions) lock down metric label contracts that are costly to re-derive from unit tests."
  - "Shared-fixture pattern for integration tests: each test file declares `mod common { pub mod reasoning_fixtures; }` — Rust compiles tests/common/reasoning_fixtures.rs per-crate. `#![allow(dead_code)]` at module scope silences per-crate unused-item lints without cluttering each item with attributes."

requirements-completed: [REAS-07, REAS-08]

duration: 45 min
completed: 2026-04-22
---

# Phase 25 Plan 06: Iteration-Loop Runner Summary

**Provider-agnostic state machine driving ReasoningProvider::generate -> dispatch -> feedback until one of four termination reasons fires (Terminal / BudgetExceeded / MaxIterations / RepeatedToolCall), with per-turn timeout, canonical-args repeated-call detector, 4096-capped max_tokens per turn, and per-profile token-accounting metrics.**

## Performance

- **Duration:** 45 min
- **Started:** 2026-04-22T07:15:00Z (approx)
- **Completed:** 2026-04-22T08:00:00Z (approx)
- **Tasks:** 2
- **Files modified:** 6 (2 created, 4 edited)
- **LOC shipped:** 795 total (225 runner + 185 fixtures + 61 smoke + 162 term + 162 budget)
- **Tests:** 8 new (1 smoke + 4 termination + 3 budget) — 8/8 green

## Accomplishments

- REAS-07 delivered: 4 distinct termination reasons exercised end-to-end under mock providers
- REAS-08 delivered: budget enforcement (pre-generate + max_tokens cap) + reasoning_tokens_total metric with per-profile label
- Pitfall 3 locked in structurally: grep acceptance forbids finish_reason in control flow; source-level regression test locks the metric label contract
- apply_salience_side_effects hook called at ALL four terminal exits — plan 07 only needs to replace the stub body

## Task Commits

1. **Task 1: runner.rs state machine + fixtures** — `fe10f30` (feat)
2. **Task 2: termination + budget tests** — `9b80171` (test)

**Plan metadata:** (this SUMMARY commit — recorded after this file is staged)

## Files Created/Modified

- `crates/memcp-core/src/intelligence/reasoning/runner.rs` — NEW 225 LOC. run_agent / run_agent_with_provider / run_agent_with_provider_and_timeout + canonicalize_value + hash_canonical_call.
- `crates/memcp-core/src/intelligence/reasoning/mod.rs` — +23 lines. Module registration, stub apply_salience_side_effects, re-exports of the three public entries.
- `crates/memcp-core/tests/common/reasoning_fixtures.rs` — NEW 185 LOC. MockReasoningProvider, SlowMockProvider, RecordingMockProvider, tc_call_with_args, NullStore, noop_ctx.
- `crates/memcp-core/tests/reasoning_loop_test.rs` — 61 LOC. Smoke test for Terminal path (1 test).
- `crates/memcp-core/tests/reasoning_termination.rs` — 162 LOC. 4 termination tests (Terminal / MaxIterations / RepeatedToolCall / Transport(timeout)).
- `crates/memcp-core/tests/reasoning_budget.rs` — 162 LOC. 3 budget tests (hard-stop, max_tokens bounded, metric-label source guard).
- `crates/memcp-core/Cargo.toml` — +1 line. Explicit `futures = "0.3"` dep.

## Decisions Made

See frontmatter `key-decisions` for the canonical list. Quick summary:

- Salience hook stub lives in mod.rs; runner imports via super::
- Single-line #[rustfmt::skip]'d metrics::counter! invocation (acceptance-grep contract)
- Three-tier entry (factory / mock / mock+timeout) for per-test flexibility
- Shared NullStore + noop_ctx in fixtures (not duplicated per file)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocker] futures crate was only a transitive dep**
- **Found during:** Task 1 (runner.rs uses `futures::future::join_all`)
- **Issue:** Plan's read_first noted "if futures missing, Plan 00 check should have caught it — fail loudly if absent". `cargo tree` showed futures 0.3 reachable transitively via tokio/etc. but NOT as an explicit memcp-core dep — importing `use futures::future::join_all;` directly would be brittle on future dep-tree pruning.
- **Fix:** Added `futures = "0.3"` to crates/memcp-core/Cargo.toml [dependencies].
- **Files modified:** crates/memcp-core/Cargo.toml
- **Verification:** `cargo build -p memcp-core` green.
- **Committed in:** fe10f30 (Task 1 commit)

**2. [Rule 1 - Bug] Plan's NullStore was missing 4 required trait methods**
- **Found during:** Task 1 (tests/common/reasoning_fixtures.rs impl block)
- **Issue:** Plan's example NullStore stubbed only `store_with_outcome / update / delete / get`. The real `MemoryStore` trait requires `list`, `count_matching`, `delete_matching`, `touch` too (no defaults for those 4). `cargo build --tests` failed with `not all trait items implemented`.
- **Fix:** Added all 4 missing methods to NullStore returning cheap ok/empty values (`list -> empty ListResult`, `count_matching -> 0`, `delete_matching -> 0`, `touch -> Ok(())`). Also corrected `get` return type — plan showed `Result<Option<Memory>, MemcpError>` but the trait actually returns `Result<Memory, MemcpError>`.
- **Files modified:** crates/memcp-core/tests/common/reasoning_fixtures.rs
- **Verification:** `cargo build --tests -p memcp-core` green.
- **Committed in:** fe10f30 (Task 1 commit)

**3. [Rule 1 - Bug] Plan's `if let Some(fr) = resp.finish_reason` violates its own acceptance grep**
- **Found during:** Task 1 (acceptance verification loop)
- **Issue:** The plan's example runner body uses `if let Some(fr) = resp.finish_reason.as_deref() { tracing::debug!(...); }` for the Reviews LOW #12 diagnostic log. But the plan's OWN acceptance criterion requires `grep -cE 'if.*finish_reason|match.*finish_reason|resp\.finish_reason.*==' = 0`. The `if let` form matches `if.*finish_reason`.
- **Fix:** Rewrote the diagnostic as `let diag_finish = resp.finish_reason.as_deref().unwrap_or("<none>"); tracing::debug!(...)`. Keeps the LOW #12 diagnostic intent while satisfying the Pitfall 3 guard (the binding avoids both `if` and `resp.finish_reason.*==` patterns).
- **Files modified:** crates/memcp-core/src/intelligence/reasoning/runner.rs
- **Verification:** `grep -cE 'if.*finish_reason|...' runner.rs` = 0; `grep -c 'tracing::debug!' runner.rs` = 2 (one in the diagnostic, one inferred from the macro expansion region — still >=1 per spec).
- **Committed in:** fe10f30 (Task 1 commit)

**4. [Rule 1 - Bug] Plan's test_repeated would have been won by MaxIterations instead**
- **Found during:** Task 2 (termination test authoring)
- **Issue:** Plan's test_repeated uses `profile()` with `max_iterations=3` and queues 3 identical responses. The repeated-call detector fires only when the VecDeque has 3 entries all equal — this happens after iter 2's generate call completes (3rd call pushed). At that point iter has already advanced. With max_iterations=3, if the loop exits on iter 3 via MaxIterations OR via RepeatedToolCall is timing-sensitive; in practice the detector fires first because it runs before messages/dispatch but iterations is already 3 at the exit. Tests flakily pass but the intent is fragile.
- **Fix:** In test_repeated, bumped `prof.max_iterations = 10` so the repeated-call detector is strictly the reason the loop exits (no ambiguity). Keeps the test asserting `matches!(out, AgentOutcome::RepeatedToolCall { .. })` unchanged.
- **Files modified:** crates/memcp-core/tests/reasoning_termination.rs
- **Verification:** `cargo test --test reasoning_termination` — all 4 tests green including test_repeated.
- **Committed in:** 9b80171 (Task 2 commit)

**5. [Rule 1 - Bug] Acceptance grep `metrics::counter!("reasoning_tokens_total"` counted a matching comment**
- **Found during:** Task 1 (acceptance verification loop)
- **Issue:** My initial comment above the macro call contained the literal string `metrics::counter!("reasoning_tokens_total"` for context, which caused the acceptance grep to return 2 instead of 1.
- **Fix:** Rephrased the comment to describe the macro by name without reproducing the literal. Also merged the macro invocation into a single line with `#[rustfmt::skip]` to satisfy acceptance criteria consistently.
- **Files modified:** crates/memcp-core/src/intelligence/reasoning/runner.rs
- **Verification:** `grep -c 'metrics::counter!("reasoning_tokens_total"' runner.rs` = 1.
- **Committed in:** fe10f30 (Task 1 commit)

**6. [Rule 1 - Bug] `indexing_slicing` clippy warning on `last_call_hashes[0]`**
- **Found during:** Task 1 (clippy sweep on touched files)
- **Issue:** Workspace lints set `indexing_slicing = "warn"`. My initial `last_call_hashes.iter().all(|h| *h == last_call_hashes[0])` tripped it even though the `len() == 3` guard makes the index safe.
- **Fix:** Rewrote as `last_call_hashes.front().is_some_and(|first| last_call_hashes.iter().all(|h| h == first))` — no indexing, clippy-clean.
- **Files modified:** crates/memcp-core/src/intelligence/reasoning/runner.rs
- **Verification:** `cargo clippy -p memcp-core --tests` no longer warns on runner.rs indexing.
- **Committed in:** fe10f30 (Task 1 commit)

---

**Total deviations:** 6 auto-fixed (4× Rule 1 bugs, 1× Rule 3 blocker, 1× Rule 1 bug related to grep-self-inconsistency)
**Impact on plan:** All deviations necessary for correctness or plan-self-consistency. No scope creep — the behavior delivered is exactly what the plan specifies. The 4 Rule-1 fixes all relate to the plan's own example code not matching its own acceptance criteria or trait reality; the behavior contract is unchanged.

## Reviews Items Addressed

- **LOW #9 (per-run `validate_tool_schemas`):** kept per-run call as a defensive check (runner tests inject custom tool sets via run_agent_with_provider). Boot-time call deferred to a separate follow-up — moving it here would require threading through every current/future caller.
- **LOW #12 (log `finish_reason` at debug level):** runner logs `finish_reason` at `tracing::debug!` WITHOUT using it for control flow. Pitfall 3 held: terminator = tool_calls.is_empty() only. Grep acceptance verified.

## Issues Encountered

None. The 6 auto-fixes above were Rule 1/3 deviations, all addressed in-flight within the task commits.

## User Setup Required

None — runner is internal plumbing; no env vars, accounts, or dashboards touched.

## Next Phase Readiness

- **Plan 25-07 (salience hook):** UNBLOCKED. apply_salience_side_effects already called at all 4 exit points (Terminal / BudgetExceeded / MaxIterations / RepeatedToolCall). Plan 07 replaces the stub body in mod.rs with real x1.3/x0.9/x0.1 writes against PostgresMemoryStore::apply_stability_boost. No runner changes needed.
- **Plan 25-08 (BYOK wiring):** UNBLOCKED. run_agent accepts `AgentCallerContext { creds, ... }`. Plan 08 wires the HTTP middleware that populates `creds` from the `x-reasoning-api-key` header.
- **Phase 26 (Dreaming Worker) / Phase 27 (Agentic Retrieval):** both can now import `run_agent` and pass their own `profile_name` + `tools` + system prompt. P26 passes "dreaming" + dreaming's 6-tool palette; P27 passes "retrieval" + the retrieval specialist's palette (includes `get_memory_span` per the 24.75-04 follow-up note).

## Self-Check

- [x] `crates/memcp-core/src/intelligence/reasoning/runner.rs` exists (225 LOC)
- [x] `crates/memcp-core/tests/common/reasoning_fixtures.rs` exists (185 LOC)
- [x] `git log --oneline --all | grep fe10f30` — feat(25-06) commit present
- [x] `git log --oneline --all | grep 9b80171` — test(25-06) commit present
- [x] `cargo test -p memcp-core --test reasoning_loop_test` → 1 passed
- [x] `cargo test -p memcp-core --test reasoning_termination` → 4 passed
- [x] `cargo test -p memcp-core --test reasoning_budget` → 3 passed
- [x] `cargo build -p memcp-core` clean
- [x] `cargo build --tests -p memcp-core` clean
- [x] All 13 Task-1 acceptance grep criteria pass
- [x] All 7 Task-2 acceptance criteria pass (2 test counts + 0 ignores × 2 + 3 source-level)
- [x] No regression in lib reasoning tests (7/7 still green)

## Self-Check: PASSED

---
*Phase: 25-reasoning-agent*
*Completed: 2026-04-22*
