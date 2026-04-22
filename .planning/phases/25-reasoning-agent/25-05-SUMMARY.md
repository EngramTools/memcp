---
phase: 25-reasoning-agent
plan: 05
subsystem: reasoning
tags: [rust, memory-tools, dispatch, jsonschema, postgres, reasoning, tool-calling]

requires:
  - phase: 25-00
    provides: is_source_of_any_derived primitive + jsonschema 0.46 dep + Wave 0 scaffolds
  - phase: 25-01
    provides: ReasoningProvider trait + Tool/ToolCall/ToolResult unified types + AgentCallerContext
provides:
  - memory_tools() palette (6 tools) wired for generate() + dispatch_tool()
  - dispatch_tool() with per-call JSON Schema validation and structured-JSON error envelopes
  - MemoryStore::add_annotation trait method with Postgres jsonb_set impl
  - Reviews HIGH #3, HIGH #5, MEDIUM #6, MEDIUM #7 closed
affects: [25-06-runner-loop, 25-07-salience-hook, 26-dreaming, 27-agentic-retrieval]

tech-stack:
  added: []
  patterns:
    - per-call jsonschema validator_for + serde double-barrel (validator fails → schema_validation; typed deserialize fails → bad_args)
    - structured-JSON error envelopes ({"error","code"}) returned via is_error=true instead of plain text
    - trait-level escape-hatch fields (force_if_source) that emit tracing::warn! AND surface a warning field in the ToolResult so agents self-correct
    - agent-first dispatch through MemoryStore trait only — no direct Postgres in tool handlers

key-files:
  created:
    - crates/memcp-core/src/intelligence/reasoning/tools.rs
  modified:
    - crates/memcp-core/src/intelligence/reasoning/mod.rs
    - crates/memcp-core/src/storage/store/mod.rs
    - crates/memcp-core/src/storage/store/postgres/extraction.rs
    - crates/memcp-core/src/storage/store/postgres/queries.rs
    - crates/memcp-core/tests/reasoning_tool_dispatch.rs
    - crates/memcp-core/tests/reasoning_tools_test.rs

key-decisions:
  - "search_memories delegates to MemoryStore::list + in-memory substring filter for the MVP — the plan's referenced recall::recall(store, query, limit, tier) free function does not exist; real RecallEngine::recall needs an embedding and is bound to concrete PostgresMemoryStore. Hybrid search via the dispatcher is deferred to Phase 27 agentic retrieval."
  - "add_annotation UPDATE binds id with plain $1 (no ::uuid cast) to match the pattern already used by queries.rs delete/update/touch — the ::uuid cast in the plan's literal SQL caused a silent 'Database operation failed' on live DB until removed."
  - "select_final_memories also removes ids from read_but_discarded so a final-selected id never double-accounts as discarded-but-selected."
  - "create_memory also pushes source_ids into final_selection so Plan 25-07 stability boost flows through the provenance graph (not just the terminal memory)."

patterns-established:
  - "Two-barrel arg validation: jsonschema::validator_for on tool.parameters BEFORE serde_json::from_value — distinguishes schema_validation errors (LLM malformed) from bad_args (type mismatch) in the agent's feedback loop"
  - "Per-call schema validation flow: find tool → compile validator → validate arguments → typed deserialize → dispatch. Defensive outer match for find failures (unknown_tool) and schema compile failures (bad_tool_schema)"
  - "Structured JSON error envelope helper err_result(call, code, msg) — every error path routes through the single helper so agents always see {\"error\",\"code\"} (MEDIUM #7 enforced at the call-site level, not by convention)"
  - "Escape-hatch pattern: boolean flag (force_if_source) bypasses safety guard, emits tracing::warn! at operator level, AND emits warning field in the ToolResult so the agent's own context has the escape-hatch signal for self-correction"

requirements-completed: [REAS-06]

duration: 35min
completed: 2026-04-22
---

# Phase 25 Plan 05: Memory Tools + Dispatcher Summary

**6-tool palette (search/create/update/delete/annotate/select_final) with per-call jsonschema validation, structured-JSON error envelopes, D-06 cascade guard + force_if_source escape hatch, and Postgres `jsonb_set` add_annotation backing — all 4 Review gates closed.**

## Performance

- **Duration:** 35 min
- **Started:** 2026-04-22T06:17:57Z
- **Completed:** 2026-04-22T06:53:11Z
- **Tasks:** 2/2
- **Files modified:** 7 (1 created, 6 modified)

## Accomplishments

- `memory_tools()` returns Vec<Tool> of exactly 6 entries with canonical Phase 24 `knowledge_tier` enum (`[raw, imported, explicit, derived, pattern]`) — no `episodic`/`semantic` drift (HIGH #3)
- `dispatch_tool` runs `jsonschema::validator_for(&tool.parameters).validate(&call.arguments)` BEFORE `serde_json::from_value`, returning structured-JSON errors with distinct `code` values (`schema_validation`, `bad_args`, `storage_error`, `unknown_tool`, `cascade_delete_forbidden`, `bad_tool_schema`) — MEDIUM #6 + #7 closed
- `delete_memory` fires `MemoryStore::is_source_of_any_derived` BEFORE delete (Phase 24 D-06); `force_if_source=true` bypasses with `tracing::warn!` + `warning` field in `ToolResult` — HIGH #5 closed
- New `MemoryStore::add_annotation` trait method (default returns `Internal("unimpl")`) + Postgres inherent impl via `jsonb_set(metadata, '{annotations}', … || to_jsonb($2::text))` + `updated_at = NOW()` bump; LOW #11 side-effect comment in source
- `validate_tool_schemas` iterates the palette and surfaces first bad schema as `ReasoningError::BadToolSchema(name, reason)` for server-startup detection
- `select_final_memories(ids)` populates `AgentCallerContext.final_selection` for Plan 25-07 stability boost; `create_memory` also inserts `source_ids` into `final_selection` so provenance nodes flow through the boost
- 4 in-crate unit tests (`tool_schema_tests`) + 8 DB-gated integration tests (`reasoning_tool_dispatch.rs`) — all GREEN against live dev Postgres

## Task Commits

1. **Task 1: add_annotation trait + Postgres impl** — `e465984` (feat)
2. **Task 2: 6 tools + dispatch_tool + tests** — `de53797` (feat)

_Plan metadata (SUMMARY + STATE) committed separately as docs(25-05)._

## Files Created/Modified

- `crates/memcp-core/src/intelligence/reasoning/tools.rs` — **NEW** 6 tool defs + dispatch_tool + 4 unit tests (~490 lines)
- `crates/memcp-core/src/intelligence/reasoning/mod.rs` — registered `tools` module, re-exported `memory_tools`, `dispatch_tool`, `validate_tool_schemas`
- `crates/memcp-core/src/storage/store/mod.rs` — added `add_annotation` default trait method
- `crates/memcp-core/src/storage/store/postgres/extraction.rs` — `pub async fn add_annotation` inherent impl (jsonb_set append + updated_at bump + LOW #11 side-effect comment)
- `crates/memcp-core/src/storage/store/postgres/queries.rs` — trait forwarder routes `&dyn MemoryStore::add_annotation` to the inherent impl
- `crates/memcp-core/tests/reasoning_tools_test.rs` — Wave 0 scaffold flipped GREEN (palette size + schema validity smoke test)
- `crates/memcp-core/tests/reasoning_tool_dispatch.rs` — 8 DB-gated integration tests replacing Wave 0 stubs

## Test Counts

| Surface | Tests | Status |
|-|-|-|
| `lib tool_schema_tests` (unit) | 4 | 4 pass |
| `reasoning_tools_test` (integration, always-on) | 1 | 1 pass |
| `reasoning_tool_dispatch` (integration, DB-gated) | 8 | 8 pass against `postgres://memcp:memcp@localhost:5433/memcp` (short-circuit no-op when env unset) |

**Verification commands:**
```bash
cargo build -p memcp-core                                   # exit 0
cargo test -p memcp-core --lib tool_schema_tests            # 4 pass
cargo test -p memcp-core --test reasoning_tools_test        # 1 pass
MEMCP_TEST_DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp \
  cargo test -p memcp-core --test reasoning_tool_dispatch   # 8 pass
```

## Decisions Made

- **search_memories MVP via `list()` + substring filter.** The plan imagined `recall::recall(store, query, limit, tier)` as a free function; the real `RecallEngine::recall` takes an embedding and is method-bound to concrete `PostgresMemoryStore`. Extending `MemoryStore` to take a text query would push embedding generation into the trait — deferred. The dispatcher keeps the agent-first rule (no direct Postgres) and returns reasonable results via `list()` + in-memory substring filter. Phase 27 retrieval specialist will use `hybrid_search` through the concrete store.
- **`create_memory` pushes `source_ids` into `final_selection`.** Consistency with the REAS-10 stability-boost contract — provenance nodes flow through the boost too.
- **`select_final_memories` removes ids from `read_but_discarded`.** Prevents the same id from counting as both "discarded" and "final" in downstream bookkeeping.
- **`$1` not `$1::uuid` in add_annotation UPDATE.** Matches the binding pattern in `queries.rs` delete/update/touch; the `::uuid` cast produced a sanitized "Database operation failed" on the live dev DB until removed.
- **No-op short-circuit when `MEMCP_TEST_DATABASE_URL` unset.** `setup()` returns `None`, each test returns early — `cargo test` on a dev machine without Postgres still passes (per plan verification note).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 — Blocking] `recall::recall` free function does not exist**
- **Found during:** Task 2 (`search_memories` dispatch arm)
- **Issue:** Plan referenced `crate::intelligence::recall::recall(store, query, limit, tier)` as a free async function. The real module exports `RecallEngine::recall(&self, query_embedding: &[f32], session_id, reset, project, boost_tags)` bound to concrete `PostgresMemoryStore`. The signature mismatch blocks the planned dispatch.
- **Fix:** Deviated to `MemoryStore::list` + in-memory substring filter on content. Preserves agent-first rule (dispatch through trait, not concrete store). Captures the same shape downstream (`{id, content_snippet (200 chars), tier}`).
- **Files modified:** `crates/memcp-core/src/intelligence/reasoning/tools.rs`
- **Verification:** Palette + per-call schema + all passing tests still exercise `search_memories` path compilation; DB-gated tests verify the dispatcher matches the tool-result contract for neighbouring tools.
- **Committed in:** `de53797`

**2. [Rule 1 — Bug] StoreOutcome variants mismatch**
- **Found during:** Task 2 (create_memory dispatch arm)
- **Issue:** Plan's handler pattern matched `StoreOutcome::Stored(m)` and `StoreOutcome::Deduplicated { existing_id, .. }` — the actual enum is `StoreOutcome::Created(Memory) | Deduplicated(Memory)` with a convenience `memory()` accessor.
- **Fix:** Use `outcome.memory().id.clone()` — works for both variants without pattern matching.
- **Files modified:** `crates/memcp-core/src/intelligence/reasoning/tools.rs`, `crates/memcp-core/tests/reasoning_tool_dispatch.rs`
- **Verification:** 8 integration tests pass against live DB.
- **Committed in:** `de53797`

**3. [Rule 1 — Bug] `MemcpError::NotFound` shape mismatch**
- **Found during:** Task 1 (Postgres add_annotation)
- **Issue:** Plan's handler used `MemcpError::NotFound(format!("memory {id}"))` — tuple variant. Real variant is `NotFound { id: String }` struct variant.
- **Fix:** Use `MemcpError::NotFound { id: format!("memory {id}") }`.
- **Files modified:** `crates/memcp-core/src/storage/store/postgres/extraction.rs`
- **Verification:** `cargo build -p memcp-core` exits 0.
- **Committed in:** `e465984`

**4. [Rule 1 — Bug] `PostgresMemoryStore::connect` does not exist**
- **Found during:** Task 2 (integration test setup())
- **Issue:** Plan's test used `PostgresMemoryStore::connect(&db_url)` — actual constructor is `PostgresMemoryStore::new(&db_url, run_migrations: bool)`.
- **Fix:** Use `PostgresMemoryStore::new(&db_url, false)` (we don't want migrations to run on the test DB during Plan 05 — assume it's already migrated).
- **Files modified:** `crates/memcp-core/tests/reasoning_tool_dispatch.rs`
- **Verification:** 8/8 integration tests pass against live DB.
- **Committed in:** `de53797`

**5. [Rule 1 — Bug] Crate library name is `memcp` not `memcp_core`**
- **Found during:** Task 2 (integration tests)
- **Issue:** Plan's test imports used `memcp_core::intelligence::reasoning::…` — the `memcp-core` crate re-names its library target to `memcp` via `[lib] name = "memcp"` (consistent with every other `reasoning_*` integration test).
- **Fix:** Changed imports to `memcp::intelligence::reasoning::…` and `memcp::store::…`.
- **Files modified:** `crates/memcp-core/tests/reasoning_tools_test.rs`, `crates/memcp-core/tests/reasoning_tool_dispatch.rs`
- **Verification:** Both test binaries link.
- **Committed in:** `de53797`

**6. [Rule 1 — Bug] `CreateMemory` has no `Default` impl**
- **Found during:** Task 2 (build_create_memory + sample_create helpers)
- **Issue:** Plan used `CreateMemory { content: …, knowledge_tier: Some(...), ..Default::default() }` — `CreateMemory` does not `impl Default`. It has a mix of `#[serde(default = ...)]` attributes for String fields and `#[serde(default)]` for Option fields, but no derive.
- **Fix:** Explicit field-by-field construction in `build_create_memory` (dispatcher) and `sample_create` (tests). Uses sensible defaults (`type_hint="fact"`, `source="reasoning-agent"`, `actor_type="agent"`, `audience="global"`, `write_path=Some("reasoning_agent")`).
- **Files modified:** `crates/memcp-core/src/intelligence/reasoning/tools.rs`, `crates/memcp-core/tests/reasoning_tool_dispatch.rs`
- **Verification:** Build + all tests green.
- **Committed in:** `de53797`

**7. [Rule 1 — Bug] `$1::uuid` cast caused silent DB error**
- **Found during:** Task 2 (first test run against live DB — `test_annotate_memory_appends` failed with sanitized "Database operation failed")
- **Issue:** Plan's UPDATE used `WHERE id = $1::uuid`. Under the live dev DB, this produced a storage error that sanitizes to generic "Database operation failed". `queries.rs` delete/update/touch all bind `id` with plain `$1` and no cast.
- **Fix:** Removed the `::uuid` cast. `$2::text` cast on the annotation payload retained per plan acceptance criterion.
- **Files modified:** `crates/memcp-core/src/storage/store/postgres/extraction.rs`
- **Verification:** All 8 DB-gated integration tests pass post-fix.
- **Committed in:** `de53797`

### Acceptance-Criteria Clarifications (not deviations)

- `grep -cE '"episodic"|"semantic"' tools.rs returns 0` — the plan's grep will hit the in-file HIGH #3 guard assertion `for forbidden in &["episodic", "semantic"]` (1 match). The *semantic* contract — no forbidden values in actual tool schemas — is enforced by the passing `knowledge_tier_enum_uses_canonical_phase24_values` test. Keeping the guard raises match count to 1; counting the guard as a positive, not a regression.
- `grep -cE '"code": "..."' tools.rs returns ≥3` — my source uses the `err_result(call, "code_str", msg)` helper, not inline `"code": "…"` literals. The semantic contract (every error surfaces a `code` field) is enforced by `err_result` which is the only error-producing path. Error-code literal counts (17) exceed the plan's ≥3 threshold by the intended measure.

---

**Total deviations:** 7 auto-fixed (1 Rule 3 blocking, 6 Rule 1 bugs)
**Impact on plan:** All deviations mechanical corrections to stale API references in the plan — no architectural change, no new trait methods beyond the planned `add_annotation`, no new dependencies. The `search_memories` MVP is a scope trim (substring filter vs hybrid search) and is documented inline as a Phase 27 pickup.

## Issues Encountered

- First run of `test_annotate_memory_appends` against the live dev DB failed with a sanitized error until the `$1::uuid` cast was removed from the UPDATE statement. Root cause: casting in the WHERE clause clashes with sqlx's default bind type for `&str` on the `memories.id` column (which is already UUID — no cast needed on input).

## User Setup Required

None — no external service configuration required. Running DB-gated integration tests requires:
```bash
export MEMCP_TEST_DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp
```
(Optional — tests short-circuit to no-op when env unset.)

## Next Phase Readiness

- **Plan 25-06 (runner loop) unblocked.** `dispatch_tool` signature is stable: `async fn dispatch_tool(call: &ToolCall, ctx: &AgentCallerContext) -> ToolResult`. Plan 06's `join_all(dispatch_tool(&c, &ctx) for c in tool_calls)` parallel pattern will work as designed.
- **Plan 25-07 (salience hook) unblocked.** `AgentCallerContext.final_selection` is populated correctly by both `select_final_memories` AND `create_memory` (source_ids flow-through).
- **Reviews HIGH #3, HIGH #5, MEDIUM #6, MEDIUM #7 closed.** LOW #10 (snippet length 200 vs AI-SPEC 500) deferred to doc-sync pass as planned. LOW #11 (updated_at side effect) documented inline in `extraction.rs`.
- **Known follow-up:** `search_memories` in the dispatcher is MVP substring filter. Phase 27 agentic retrieval will extend the retrieval specialist's tool palette to invoke `hybrid_search` via the concrete store (RESEARCH §Retrieval Domain).

## Self-Check: PASSED

Verified against working tree:
- FOUND: `crates/memcp-core/src/intelligence/reasoning/tools.rs`
- FOUND: commit `e465984` (Task 1: add_annotation trait + Postgres impl)
- FOUND: commit `de53797` (Task 2: 6 tools + dispatch_tool + tests)
- FOUND: `crates/memcp-core/tests/reasoning_tool_dispatch.rs` replaced with 8 integration tests
- FOUND: `crates/memcp-core/tests/reasoning_tools_test.rs` flipped GREEN
- `MemoryStore::add_annotation` trait default present (mod.rs)
- `PostgresMemoryStore::add_annotation` inherent impl present (extraction.rs)
- Trait forwarder present (queries.rs, line ~790)
- `cargo build -p memcp-core` exits 0
- `cargo test -p memcp-core --lib tool_schema_tests` → 4 pass
- `cargo test -p memcp-core --test reasoning_tools_test` → 1 pass
- `cargo test -p memcp-core --test reasoning_tool_dispatch` with DB URL → 8 pass

---
*Phase: 25-reasoning-agent*
*Completed: 2026-04-22*
