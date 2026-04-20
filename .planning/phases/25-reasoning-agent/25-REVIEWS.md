---
phase: 25
reviewers: [gemini-2.5-pro, gpt-5]
reviewed_at: 2026-04-20T14:00:00Z
plans_reviewed: [25-00-PLAN.md, 25-01-PLAN.md, 25-02-PLAN.md, 25-03-PLAN.md, 25-04-PLAN.md, 25-05-PLAN.md, 25-06-PLAN.md, 25-07-PLAN.md, 25-08-PLAN.md]
skipped: [claude (self), codex (not installed), coderabbit (not installed), qwen (not installed), cursor (not installed)]
---

# Cross-AI Plan Review — Phase 25 (Reasoning Agent)

## Gemini 2.5 Pro Review

This is an exceptionally thorough and well-structured set of plans. The diligent use of in-repo patterns, clear separation of concerns, and security-first mindset are commendable. The overall approach is robust and demonstrates a mature development process.

### Summary

The plan for Phase 25 is a comprehensive, low-risk, and well-designed strategy to introduce a core reasoning capability into `memcp`. It correctly isolates provider-specific logic, separates transport-layer concerns from the core agent loop, and builds on established project patterns. The phased rollout of adapters and the detailed, test-driven execution plans minimize risk. While the overall plan is excellent, there are minor opportunities to improve robustness around edge cases in tool handling and to optimize startup-time validations.

### Strengths

- **Pattern-Driven Development:** The explicit mapping of new files to existing project analogs (`25-PATTERNS.md`) is a standout practice that ensures consistency and reduces implementation risk.
- **Excellent Separation of Concerns:** The architecture correctly isolates the iteration loop (`runner.rs`), vendor-specific adapters (`kimi.rs`, etc.), and credential handling (transport middleware), making the system more modular and maintainable.
- **Security-First Hardening:** The BYOK vs. Pro credential boundary is a critical security surface. Plan `25-08`'s design to have Pro-tier middleware silently strip caller-supplied keys is the correct, hardened approach. This design, coupled with dedicated security tests, effectively mitigates Critical Failure Mode #5.
- **Robust Loop Design:** The runner design correctly identifies and handles multiple termination conditions (terminal message, budget exceeded, max iterations, repeated calls), avoiding common pitfalls of agent loops. Using `tool_calls.is_empty()` as the sole terminator signal is the right choice, given provider inconsistencies.
- **Pragmatic Phased Rollout:** Shipping the core trait with only three adapters (per D-02) is a smart way to de-risk the project. It allows the core `ReasoningProvider` API to be validated by real downstream consumers (Phases 26 & 27) before investing in the full set of adapters.
- **Comprehensive Testing Strategy:** The validation plan is thorough, covering unit, integration, and security boundary tests. The Wave 0 creation of RED test scaffolds is an excellent TDD practice.

### Concerns

- **(HIGH) Inflexible `delete_memory` Guard:** The D-06 cascade-delete guard, as planned, is a safe default but may be too rigid for the agent. If an agent determines a derived memory is flawed and wishes to delete it *and then* its source, the current design would force it into a complex, multi-turn, potentially stateful operation. An agent cannot express its intent to override the safety check, which could lead to it getting "stuck".
- **(MEDIUM) Race Condition in Salience Audit:** The `apply_stability_boost` implementation in Plan `25-00` reads the current stability, calculates the new value, and then writes it back within a transaction. If two concurrent runs target the same memory, both could read the same initial `prev_stability`, leading to an incorrect `prev_stability` value being logged in the audit trail for the second transaction, even though the final stability score would be correct.
- **(LOW) `annotate_memory` Side Effect:** Plan `25-05`'s implementation of `annotate_memory` via `jsonb_set` will update the `updated_at` timestamp on the parent `memories` row. This is a subtle side effect, as adding metadata might not be considered a "content" update. Downstream systems that rely on `updated_at` to track content modifications could be triggered unexpectedly.
- **(LOW) Per-Instance Ollama Capability Caching:** The `AtomicBool` cache for the Ollama capability probe in Plan `25-04` is scoped to the provider instance. In high-concurrency scenarios where provider instances may be created per-request, this could lead to redundant `/api/show` calls.

### Suggestions

- **For `delete_memory` Guard:** Add an optional boolean argument to the `delete_memory` tool: `delete_memory(id: string, force_if_source?: boolean)`. Default it to `false`. The tool's description must clearly warn that `force_if_source: true` bypasses the D-06 safety check and could leave orphaned derived memories, giving the agent an explicit escape hatch for advanced use cases.
- **For Salience Audit:** The risk is low and only affects the audit log's historical accuracy, not the live score. The simplest resolution is to add a code comment to the `apply_stability_boost` function acknowledging the minor race condition on the `prev_stability` value in the audit log under high concurrency.
- **For `annotate_memory`:** The current implementation is pragmatic. Add a clarifying comment in the `add_annotation` function in `storage/store/postgres/extraction.rs` noting that this operation updates the parent memory's `updated_at` timestamp.
- **For Runner Validation:** The `validate_tool_schemas` function in Plan `25-05` is called by the runner on every invocation. Since the toolset is static, this check can be moved to a one-time server startup routine. This would make it a boot-time failure and remove the marginal per-run overhead.

### Risk Assessment

**Overall Risk: LOW**

The project is exceptionally well-planned, mitigating most common risks associated with building agentic systems.

- **Technical Risk: Low.** The heavy reliance on existing, proven patterns within the codebase significantly reduces the risk of architectural flaws. The language choice (Rust) and the clean separation of concerns further increase technical stability.
- **Execution Risk: Low.** The plans are broken down into atomic, verifiable steps with clear dependencies and acceptance criteria. The TDD-centric approach with pre-staged RED tests ensures a smooth, predictable implementation flow.
- **Security Risk: Low.** The most critical security risk (BYOK credential leakage) is identified and addressed with a robust, test-verified middleware design. The security domain analysis is thorough.

---

## GPT-5 Review

**Phase 25 Review — Reasoning Agent**

### Summary

The plan set is ambitious but coherent. It mirrors proven in‑repo patterns (trait+factory+adapters), cleanly separates provider quirks from a unified tool-calling loop, and threads security, cost, and salience feedback into the design. Validation is unusually thorough: adapter wiremock tests, runner termination/budget tests, transport BYOK boundary tests, and DB‑backed salience tests. Biggest risks are a few contract inconsistencies (knowledge_tier enums), a non‑idempotent salience audit story vs earlier "idempotent per‑run" claims, an annotation path that assumes a metadata column, and BYOK middleware that wrongly treats Ollama as key‑based. These are fixable with scoped deltas.

### Strengths

- Follows established memcp patterns: trait + factory + per‑adapter files; adapters translate to unified Tool/ToolCall/ToolResult.
- Loop runner covers real failure modes: budget hard stop, per‑turn timeout, repeated‑call detector, provider‑agnostic termination.
- Salience design is audit‑first (run_id), magnitude‑controlled, and separated from tools (hooked at the runner).
- Security posture is explicit: Pro strips BYOK headers; allowlist on providers; tests byte‑verify Authorization behavior.
- Test coverage is deep and actionable: wiremock for adapters, unit/integ for runner/budget/termination, DB salience checks.
- Config and profiles enable profile‑specific tuning; metrics labels use profile names (operator‑friendly).

### Concerns

- **HIGH** — Idempotency mismatch for salience: 25‑VALIDATION requires "idempotent per‑run," but 25‑07 applies boosts on every call (no (run_id, memory_id) uniqueness) and explicitly says re‑invocation will re‑multiply. This can double‑boost on retries.
- **HIGH** — BYOK middleware and Ollama: BYOK requires x‑reasoning‑api‑key for all providers; Ollama needs no key. Pro path 503s if no env key for Ollama. This blocks self‑hosted Ollama in both tenancies.
- **HIGH** — Tool schema/enum drift: tools.rs uses knowledge_tier values `["raw","imported","explicit","derived","pattern"]`, while earlier docs and parts of the plan reference `["episodic","semantic","derived"]`. Runner/tests will pass, but dispatcher/store may reject or mis-route tiers at runtime.
- **MEDIUM** — annotate_memory assumes a `metadata` JSONB column on memories; migration to add/shape that column is not listed. Missing column → runtime SQL error.
- **MEDIUM** — JSON Schema not enforced per-call: plans validate tool schemas once per run, but do not validate each ToolCall.arguments against that schema before serde‑deserialization (only typed parse). This loses min/max/enum guarantees and degrades model self‑correction loop.
- **MEDIUM** — Salience revert correctness: revert_boost updates stability once per fetched audit row then deletes all rows for run_id; multiple rows per (run_id, memory_id) produce order‑dependent end states. No unique constraint to prevent duplicates.
- **MEDIUM** — Unknown-tool/error content shape: error ToolResult.content is plain text in places; elsewhere the spec expects a JSON object string. Inconsistent error payloads can reduce model self‑repair reliability.
- **MEDIUM** — Transport wiring gaps: AppState extension (ReasoningCreds/ReasoningTenancy) requires router/boot integration; plan notes "wire in daemon boot code" but doesn't enumerate exact sites. Easy to miss → middleware runs with empty state.
- **LOW** — Performance: schema validation runs per run (not once at boot). Small but avoidable overhead; composes poorly at scale.
- **LOW** — Message history growth: no compaction in Phase 25 (accepted), but search_memories returns 200‑char snippets (not 500 per AI‑SPEC); be consistent or note rationale.

### Suggestions

1. **Fix salience idempotency** — Add a unique index on `salience_audit_log (run_id, memory_id)` and make `apply_stability_boost` UPSERT no‑op on conflict. Alternatively, skip if already boosted for (run_id, memory_id). Align with "idempotent per‑run." Update tests to verify duplicate invocations with same run_id do not double‑boost.
2. **BYOK/Ollama tenancy logic** — Special‑case `"ollama"`: never require api_key in BYOK; in Pro, absence of env key must not 503 — allow no‑auth path. Document in ReasoningMwState and tests.
3. **Unify knowledge_tier enums** — Pick the canonical Phase 24 set (codebase says `"raw/imported/explicit/derived/pattern"` per migration 026). Update all tool schemas, docs, and plan text to match. Add a quick test that create_memory rejects unknown enums.
4. **Add per‑call JSON Schema validation** — In dispatch_tool: run `jsonschema::validator_for(tool.parameters).validate(&call.arguments)` and return is_error=true with a structured JSON error. Keep typed deserialize after schema validation.
5. **Ensure annotate_memory is backed by schema** — Verify `memories.metadata` JSONB exists; if not, add a small migration to create it with a default `{}` and an index if needed. Add a test that asserts annotation round‑trips.
6. **Tighten revert_boost semantics** — If keeping multi‑row per run_id: update in descending applied_at order or aggregate to latest; better: enforce (run_id, memory_id) uniqueness and rely on idempotency per (run_id, memory_id) to simplify rollback.
7. **Normalize error payloads** — Standardize ToolResult.content errors to a JSON string like `{"error":"...","code":"constraint_violation"}`; adapters don't care, but models self‑repair better with structured hints.
8. **Move tool schema validation to startup** — Validate `memory_tools()` once at server boot; in runner, only validate if tests inject custom tools.
9. **Transport wiring completeness** — List and patch exact AppState construction and router layering sites (api/mod.rs / server.rs) in the plan with grep targets, so PR reviewers can verify the middleware is actually active.
10. **Minor correctness** — Kimi/OpenAI translate_in: ensure prior tool_result `Message::Tool` echo includes tool_call_id exactly; add a unit asserting round‑trip id echo survives multiple calls. Runner: consider logging finish_reason for diagnosis, while still ignoring it for control flow.

### Risk Assessment

**MEDIUM.** Architecture and tests are strong, and most work mirrors mature patterns. The main correctness risks (tier enum drift, Ollama BYOK/pro handling, salience idempotency) can cause user‑visible failures or silent quality drift if unaddressed. Close those three, add per‑call schema validation + annotate schema migration, and the residual risk drops to low.

---

## Consensus Summary

Two independent reviewers (Gemini 2.5 Pro, GPT-5) converge on "architecturally strong, but several concrete correctness gaps need closing before execute." Gemini rates overall risk LOW; GPT-5 rates MEDIUM — the delta is entirely about how many loose ends GPT-5 catches that Gemini treats as acceptable residuals.

### Agreed Strengths

- **Pattern reuse** — trait + factory + per-adapter separation mirrors established memcp architecture; 25-PATTERNS.md mapping singled out as unusually strong practice.
- **Loop runner robustness** — multiple termination conditions (terminal message, budget, max iterations, repeated calls) cover the real failure modes; `tool_calls.is_empty()` as sole terminator is the right call.
- **Security-first BYOK boundary** — Pro middleware stripping caller-supplied `x-reasoning-api-key` is the correct design, test-verified.
- **Test depth** — wiremock adapter tests + runner termination/budget tests + DB-backed salience tests + transport security tests are genuinely comprehensive.
- **Pragmatic scope** — shipping only Kimi/OpenAI/Ollama (D-02) to validate trait shape before scaling adapters is smart de-risking.

### Agreed Concerns (both reviewers flagged these — address before execute)

1. **Runner-level schema validation is wasteful** — both reviewers independently flag that `validate_tool_schemas` runs per-invocation when the toolset is static; move to boot-time. (Gemini LOW, GPT-5 LOW)
2. **Correctness gaps around salience audit** — Gemini flags a read-modify-write race on `prev_stability`; GPT-5 flags full idempotency mismatch vs 25-VALIDATION's "idempotent per-run" claim. The root cause is the same: no unique constraint on `(run_id, memory_id)` in `salience_audit_log`. **Fix both by adding the unique index + UPSERT.** (Gemini MEDIUM, GPT-5 HIGH)

### GPT-5-Only HIGH-Severity Findings (Gemini missed)

These are blocker-class issues Gemini did not surface — highest priority for re-plan:

- **Ollama + BYOK tenancy bug** — middleware requires api-key for all providers but Ollama needs none; Pro path 503s without env key. Blocks self-hosted Ollama in both tenancies.
- **knowledge_tier enum drift** — `tools.rs` uses `["raw","imported","explicit","derived","pattern"]` (Phase 24 canonical) but plan/docs reference `["episodic","semantic","derived"]` in places. Runtime rejection risk at store boundary.
- **`annotate_memory` missing migration** — assumes `memories.metadata` JSONB column exists; no migration in 25-00 to add it. Runtime SQL error on first call.

### Gemini-Only HIGH-Severity Finding (GPT-5 missed)

- **`delete_memory` D-06 guard rigidity** — no escape hatch for agents that genuinely need cascade delete. Suggests optional `force_if_source?: boolean` argument with warning in description.

### Divergent Views

- **Overall risk rating** — Gemini LOW, GPT-5 MEDIUM. The gap is entirely explained by GPT-5's three HIGH-severity findings (Ollama BYOK, tier enum drift, missing metadata migration). If those three are closed, both converge to LOW.
- **Salience audit severity** — Gemini treats the race as LOW (only audit-log accuracy); GPT-5 treats idempotency as HIGH (double-boost on retries). GPT-5 is correct here — 25-VALIDATION explicitly promises idempotency, so the plan contradicts its own contract.

### Recommended Re-plan Actions (prioritized)

**Must fix before execute (HIGH):**
1. Add `UNIQUE (run_id, memory_id)` index on `salience_audit_log` + UPSERT no-op in `apply_stability_boost`. Test: double-invoke same run_id does not double-boost.
2. Special-case `"ollama"` in ReasoningMwState — no api-key required in either Pro or BYOK path.
3. Audit all `knowledge_tier` references across plans/docs/tools.rs; align to canonical `["raw","imported","explicit","derived","pattern"]`. Add rejection test.
4. Add `memories.metadata` JSONB column migration to Plan 25-00 (or verify it already exists from an earlier phase).
5. Add `force_if_source?: boolean` optional arg to `delete_memory` tool with D-06 bypass warning in description.

**Should fix (MEDIUM):**
6. Per-call JSON Schema validation in `dispatch_tool` (before typed deserialize) for min/max/enum enforcement.
7. Normalize ToolResult error payloads to JSON: `{"error":"...","code":"..."}`.
8. Enumerate exact AppState/router wiring sites in Plan 25-08 with grep targets.

**Nice to have (LOW):**
9. Move `validate_tool_schemas` to server boot (not per-run).
10. Reconcile search_memories snippet length (200 vs 500 char per AI-SPEC).
11. Document Ollama capability-probe cache scope limitation.
12. Comment on `annotate_memory` → `updated_at` side-effect.
13. Log `finish_reason` for diagnostics (without using it for control flow).

---

## Next Steps

To incorporate this feedback into the plans:

```
/gsd-plan-phase 25 --reviews
```

That will re-run the planner with the above concerns injected. At minimum, close items 1–5 before `/gsd-execute-phase 25`.
