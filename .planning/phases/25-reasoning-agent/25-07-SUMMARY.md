---
phase: 25-reasoning-agent
plan: 07
subsystem: intelligence
tags: [rust, salience, side-effects, reasoning, postgres, fsrs, audit-log]

requires:
  - phase: 25-reasoning-agent-00
    provides: "PostgresMemoryStore::apply_stability_boost (idempotent per run_id,memory_id) + revert_boost + migration 029 salience_audit_log + Wave-0 scaffolds in reasoning_salience.rs"
  - phase: 25-reasoning-agent-05
    provides: "tools.rs dispatch populates ctx.final_selection with create_memory source_ids (no additional wiring needed here)"
  - phase: 25-reasoning-agent-06
    provides: "runner.rs stub call-sites at all 4 exit points (Terminal/BudgetExceeded/MaxIterations/RepeatedToolCall)"
provides:
  - "Real MemoryStore::apply_stability_boost trait method (default Internal('unimpl'); Postgres forwarder to inherent idempotent impl)"
  - "Real apply_salience_side_effects: x1.3 final_selection / x0.1 tombstoned / x0.9 discarded\\final_selection; propagates ctx.run_id unchanged"
  - "5 DB-gated integration tests for REAS-10 semantics + Reviews HIGH #1 idempotency contract"
affects: [phase-26-dreaming-worker, phase-27-agentic-retrieval]

tech-stack:
  added: []
  patterns:
    - "Snapshot-then-release Mutex locks before .await points in async loops (Send-safety)"
    - "Per-member failure tolerance: warn! + continue; only all-failure returns Err"
    - "Test idempotency at both layers: primitive (apply_stability_boost 0-rows-affected short-circuit) + hook (double-invoke no-op)"

key-files:
  created: []
  modified:
    - "crates/memcp-core/src/storage/store/mod.rs — MemoryStore::apply_stability_boost default (Internal unimpl)"
    - "crates/memcp-core/src/storage/store/postgres/queries.rs — forwarder to inherent idempotent impl"
    - "crates/memcp-core/src/intelligence/reasoning/mod.rs — real apply_salience_side_effects replaces plan-06 stub"
    - "crates/memcp-core/tests/reasoning_salience.rs — Wave 0 scaffold → 5 full integration tests"

key-decisions:
  - "Trait default returns Internal('unimpl') — matches add_annotation (plan 05) pattern so non-Postgres backends fail loudly rather than silently no-op"
  - "Snapshot final_selection / tombstoned / read_but_discarded under lock, then release before any .await (no Mutex held across await boundaries)"
  - "discarded.difference(&final_sel) runs AFTER final_selection boost so the set exclusion sees the in-memory final_selection, not DB state"
  - "apply_salience_side_effects returns Err only if every attempt failed; single-member failures log warn! + continue (T-25-07-02)"

patterns-established:
  - "Trait-level salience primitive exposed via forwarder — &dyn MemoryStore callers (the reasoning hook) hit the real idempotent Postgres query"
  - "Double-layer idempotency guard: primitive (UNIQUE + ON CONFLICT DO NOTHING + short-circuit) + integration test (same-run_id double-invoke assertion with explicit IDEMPOTENCY VIOLATION messages)"

requirements-completed: [REAS-10]

duration: 9min
completed: 2026-04-23
---

# Phase 25 Plan 07: REAS-10 Salience Side-Effects Hook Summary

**Replaces plan-06 `apply_salience_side_effects` stub with real ×1.3/×0.9/×0.1 stability boosts against the plan-00 idempotent primitive; 5 integration tests pin the ×-multipliers, reason strings, run_id propagation, revert semantics, and the Reviews HIGH #1 double-invoke no-op contract.**

## Performance

- **Duration:** 9 min
- **Started:** 2026-04-23T19:17:41Z
- **Completed:** 2026-04-23T19:26:13Z
- **Tasks:** 2 (both TDD-mode, impl-then-test ordering per plan)
- **Files modified:** 4

## Accomplishments

- Real REAS-10 salience hook wired: runner exits now bump salience for final_selection (×1.3), tombstoned (×0.1), and read_but_discarded minus final_selection (×0.9).
- `MemoryStore::apply_stability_boost` default-method added to the trait so `&dyn MemoryStore` reasoning callers hit the idempotent Postgres impl via forwarder.
- 5 Wave-0 scaffolds flipped GREEN against live dev Postgres (port 5433):
  - `test_final_selection_boost` — prev×1.3 + audit row (reason=final_selection, magnitude=1.3)
  - `test_discarded_decay` — prev×0.9 + audit row (reason=discarded)
  - `test_tombstone_penalty` — prev×0.1 clamped to floor 0.1
  - `test_idempotent_via_revert` — two distinct run_ids compound; revert_boost(run_b) restores state; audit rows for run_b wiped
  - `test_idempotent_double_invoke_same_run_id` — **Reviews HIGH #1 closed**: second invoke with identical (run_id, memory_id) is a NO-OP (stability stays at prev×1.3, NOT prev×1.69; audit row count stays at 1)
- Zero `#[ignore]` remaining in `reasoning_salience.rs`. Plan-06 regression suite still green (12 tests across runner/termination/budget/tool_dispatch).

## Task Commits

1. **Task 1: REAS-10 apply_salience_side_effects + trait forwarder** — `f6d97e3` (feat)
2. **Task 2: 5 integration tests incl. HIGH #1 idempotency** — `9516531` (test)

Impl-before-test ordering was explicit in the plan — the Wave-0 scaffolds from plan 00 *were* the RED gate; Task 1 made them reachable, Task 2 exercised them.

## Files Created/Modified

- `crates/memcp-core/src/storage/store/mod.rs` — new trait default `apply_stability_boost(id, magnitude, run_id, reason)` returning `Internal("unimpl")`. Matches `add_annotation` default pattern.
- `crates/memcp-core/src/storage/store/postgres/queries.rs` — trait forwarder delegating to the inherent `PostgresMemoryStore::apply_stability_boost` in salience.rs (the idempotent impl from plan 00).
- `crates/memcp-core/src/intelligence/reasoning/mod.rs` — stub body replaced with real impl: snapshot the three `Mutex<HashSet<String>>` tracking sets, then per-id boost loops for final_selection (×1.3) → tombstoned (×0.1) → discarded.difference(&final_sel) (×0.9). Individual failures log `warn!` + continue (T-25-07-02); returns `Err` only if all attempts failed.
- `crates/memcp-core/tests/reasoning_salience.rs` — full integration suite with setup() env-gated fixtures, stability_of / audit_count helpers, and 5 `#[tokio::test]` assertions (incl. two explicit `IDEMPOTENCY VIOLATION` panic messages).

## Decisions Made

- Trait default returns `Internal("unimpl")` rather than `Ok(())` so non-Postgres backends fail loudly (matches `add_annotation` pattern established in plan 05). Test stores for future in-memory backends will need explicit opt-in.
- Snapshot locks before `.await` (clone the HashSet inside the `lock()` closure, then drop the guard) — Send-safety + avoids holding a std Mutex across await points. This matches the snapshot pattern used in runner.rs for repeated-call detection.
- `discarded.difference(&final_sel)` computes the exclusion against the in-memory `final_selection` snapshot, not the DB. Same-run members in both sets get the ×1.3 boost only (intentional — T-25-07-01 says "don't double-count").
- Failure aggregation: attempts=0 → Ok (trivial path when all sets empty); attempts>0 and all failed → Err(Generation); anything else → Ok. This keeps the runner's "always call side-effects at exit" contract non-fatal for partial DB flakiness.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 — Symbol mismatch] `PostgresMemoryStore::connect` does not exist**
- **Found during:** Task 2 (writing setup())
- **Issue:** Plan action block wrote `PostgresMemoryStore::connect(&db_url).await.ok()` but the actual API is `::new(url: &str, skip_migrations: bool)`.
- **Fix:** Used `PostgresMemoryStore::new(&db_url, false).await.ok()` (matches the sibling pattern in `tests/reasoning_tool_dispatch.rs:42`).
- **Files modified:** `crates/memcp-core/tests/reasoning_salience.rs`
- **Verification:** Test binary compiled + 5 tests ran against live DB.
- **Committed in:** `9516531` (Task 2 commit)

**2. [Rule 1 — Missing Default impl] `CreateMemory { ..Default::default() }` does not compile**
- **Found during:** Task 2 (writing insert_seed())
- **Issue:** Plan's action block used struct update syntax with `..Default::default()`, but `CreateMemory` does NOT derive `Default` (I verified via grep; only `UpdateMemory` at line 184 does).
- **Fix:** Constructed all 18 fields explicitly via a `sample_create(content)` helper matching the pattern already used in `reasoning_tool_dispatch.rs:16-38`.
- **Files modified:** `crates/memcp-core/tests/reasoning_salience.rs`
- **Verification:** Compiled + all 5 tests green.
- **Committed in:** `9516531`

**3. [Rule 1 — Enum variant name] `StoreOutcome::Stored` does not exist**
- **Found during:** Task 2 (build error on `insert_seed`)
- **Issue:** Plan wrote `StoreOutcome::Stored(m) => Some(m.id)` but the variants are `Created` and `Deduplicated` (mod.rs:428).
- **Fix:** `StoreOutcome::Created(m) => Some(m.id)`.
- **Committed in:** `9516531`

**4. [Rule 1 — Crate name] `memcp_core::…` does not resolve inside the `memcp-core` crate's integration tests**
- **Found during:** Task 2 (first `cargo build --tests`)
- **Issue:** Plan's import paths were `memcp_core::intelligence::…` and `memcp_core::storage::store::…`, but `crates/memcp-core/Cargo.toml` names the crate `memcp` (not `memcp_core`), and it re-exports `store` at the root (not via `storage::store`). Sibling test `reasoning_tool_dispatch.rs` uses `memcp::intelligence::reasoning::…` and `memcp::store::…`.
- **Fix:** Changed imports to match the existing convention.
- **Committed in:** `9516531`

**5. [Rule 1 — Column type mismatch] `memory_salience.memory_id` is TEXT, not UUID**
- **Found during:** Task 2 (first DB run: `operator does not exist: text = uuid`)
- **Issue:** Plan's action block wrote `SELECT stability FROM memory_salience WHERE memory_id = $1::uuid` — but migration 005 declared the column TEXT, and the live dev DB confirms it (`\d memory_salience` shows `memory_id | text`). The audit log's `memory_id` IS uuid (migration 029), so casts there are correct.
- **Fix:** Dropped the `::uuid` cast on the `memory_salience` query in `stability_of()`. Kept it on `audit_count()` and audit-row SELECTs where the column is uuid.
- **Verification:** All 5 tests then passed.
- **Committed in:** `9516531`

---

**Total deviations:** 5 auto-fixed (all Rule 1 — plan symbol/type drift from actual codebase).
**Impact on plan:** Zero scope change. Every deviation aligned the test code with the already-shipped APIs from plans 00/05/06. No new design calls were made.

## Issues Encountered

None beyond the Rule 1 deviations above. The production impl (Task 1) compiled and passed regression on first try.

## Reviews Closed

- **HIGH #1 — Idempotency of apply_salience_side_effects per run_id**: CLOSED. `test_idempotent_double_invoke_same_run_id` asserts both invariants (stability stays at prev×1.3; audit row count stays at 1) with explicit `IDEMPOTENCY VIOLATION` panic messages so any future regression is self-diagnosing. Upstream primitive already carried the UNIQUE (run_id, memory_id) + ON CONFLICT DO NOTHING guard from plan 00; this plan adds the hook-level verification the Reviews revision asked for.

## User Setup Required

None — all wiring is internal.

## Next Phase Readiness

- REAS-10 complete. Runner (plan 06) now has real side-effects at every exit point; reasoning runs influence future recall without explicit agent annotation.
- Plan 25-08 (BYOK transport wiring) unblocked — independent of this plan but sequenced after.
- Phase 26 (dreaming worker) and Phase 27 (agentic retrieval) can now rely on ×0.1 tombstone decay + ×1.3 final_selection boost landing on the salience table when they invoke `run_agent`.

## Self-Check: PASSED

- `crates/memcp-core/src/storage/store/mod.rs` — trait default present (grep count 1).
- `crates/memcp-core/src/storage/store/postgres/queries.rs` — forwarder present.
- `crates/memcp-core/src/intelligence/reasoning/mod.rs` — real impl present (`for id in &final_sel` count=1, `discarded.difference(&final_sel)` count=1, idempotency notes count=3).
- `crates/memcp-core/tests/reasoning_salience.rs` — 0 `#[ignore]`, 5 tests, 9 `apply_salience_side_effects` references, HIGH #1 test with 2 `IDEMPOTENCY VIOLATION` assertions.
- Commits found: `f6d97e3` (Task 1), `9516531` (Task 2).
- DB-gated test run: 5/5 passed under `MEMCP_TEST_DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp`.
- Regression: `reasoning_loop_test` / `reasoning_termination` / `reasoning_budget` / `reasoning_tool_dispatch` all green (12 tests).

---
*Phase: 25-reasoning-agent*
*Plan: 07*
*Completed: 2026-04-23*
