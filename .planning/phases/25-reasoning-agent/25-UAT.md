---
status: complete
phase: 25-reasoning-agent
source: [25-00-SUMMARY.md, 25-01-SUMMARY.md, 25-02-SUMMARY.md, 25-03-SUMMARY.md, 25-04-SUMMARY.md, 25-05-SUMMARY.md, 25-06-SUMMARY.md, 25-07-SUMMARY.md, 25-08-SUMMARY.md]
started: 2026-04-23T19:30:00Z
updated: 2026-04-23T19:50:00Z
---

## Current Test

[testing complete]

## Tests

### 1. Cold Start Smoke Test
expected: memcp-core + memcp build clean; core reasoning test suite all green (≥15 tests across loop/termination/budget/tool_dispatch/trait); no compile or link errors.
result: pass
evidence: |
  cargo build -p memcp-core -p memcp-bin finished in 24.05s with only pre-existing sqlx-postgres future-incompat warning.
  17/17 green across reasoning_loop_test (1), reasoning_budget (3), reasoning_termination (4), reasoning_tool_dispatch (8), reasoning_trait_test (1).

### 2. Memory Tool Palette (Plan 25-05)
expected: `intelligence::reasoning::tools::memory_tools()` returns 6 tools with names exactly [search_memories, create_memory, update_memory, delete_memory, annotate_memory, select_final_memories]. Each tool's parameters schema uses the canonical Phase 24 knowledge_tier enum `[raw, imported, explicit, derived, pattern]`. `dispatch_tool` rejects malformed args with a structured JSON `{"error","code"}` envelope (codes: schema_validation, bad_args, storage_error, unknown_tool, cascade_delete_forbidden, bad_tool_schema). `delete_memory` refuses to delete a memory that is the source of a derived row unless `force_if_source=true` (HIGH #5).
result: pass
evidence: |
  4/4 lib tests under intelligence::reasoning::tools::tool_schema_tests (palette size, knowledge_tier canonical, schemas valid, force_if_source flag) + 8/8 integration tests under reasoning_tool_dispatch (structured error envelope, delete guards, derived-requires-source-ids, schema validation).

### 3. Provider Adapters (Plans 25-02/03/04)
expected: 15 wiremock-backed adapter tests pass — 5 each for Kimi, OpenAI, Ollama. Each adapter: normalizes stringified JSON tool-call args to `serde_json::Value` at translate_out (Pitfall 1), uses its own wire types (no cross-adapter coupling, Pitfall 5), and respects its transport constraints (Ollama probes `/api/show` to confirm tool support, Pitfall 6).
result: pass
evidence: |
  5/5 reasoning_kimi, 5/5 reasoning_openai, 5/5 reasoning_ollama (incl. /api/show capability probe with AtomicBool cache). 15/15 green.

### 4. Loop Runner + Salience Hook (Plans 25-06/07)
expected: 8 runner tests green (1 smoke + 4 termination + 3 budget). Runner terminates on (a) empty tool_calls, (b) budget exceeded, (c) max_iterations reached, (d) 3 consecutive identical tool calls. Metric label carries profile NAME (e.g. "dreaming"), not model id. `apply_salience_side_effects` writes audit rows with magnitudes 1.3/0.9/0.1 for final_selection/discarded/tombstoned; invoking twice with the same run_id does NOT double-multiply stability (HIGH #1 idempotency) — UNIQUE(run_id, memory_id) + ON CONFLICT DO NOTHING enforced.
result: pass
evidence: |
  8/8 runner (loop_test 1 + termination 4 + budget 3) + 5/5 salience against dev Postgres port 5433: test_final_selection_boost, test_discarded_decay, test_tombstone_penalty, test_idempotent_double_invoke_same_run_id, test_idempotent_via_revert. HIGH #1 empirically verified — second invoke with same run_id left stability at prev×1.3 (not prev×1.69).

### 5. BYOK Middleware (Plan 25-08)
expected: 8 BYOK boundary tests green. Pro tier STRIPS caller-supplied `x-reasoning-api-key` (never logs the key value itself); BYOK tier REQUIRES `x-reasoning-api-key` for non-ollama providers (401 if missing); Ollama special-case allows no key in EITHER tier (HIGH #2); unknown provider → 400; Pro + no env key for non-ollama → 503. `ProviderCredentials` populated into request extensions for downstream handler. Middleware layered AFTER `require_api_key` in router composition.
result: pass
evidence: |
  8/8 reasoning_byok_boundary: test_pro_tier_strips_caller_api_key_header, test_byok_tier_requires_headers, test_unknown_provider_rejected, test_pro_with_server_key_absent_returns_503, test_no_reasoning_header_passes_through, test_byok_extracts_caller_key, test_byok_ollama_no_api_key_required, test_pro_ollama_no_env_key_succeeds. HIGH #2 + MEDIUM #8 closed.

## Summary

total: 5
passed: 5
issues: 0
pending: 0
skipped: 0
blocked: 0

## Gaps

[none — all tests passed]
