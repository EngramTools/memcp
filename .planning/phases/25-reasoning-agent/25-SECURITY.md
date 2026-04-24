---
phase: 25
slug: reasoning-agent
status: verified
threats_total: 41
threats_closed: 41
threats_open: 0
asvs_level: 2
created: 2026-04-23
---

# Phase 25 — Security

> Per-phase security contract: threat register, accepted risks, and audit trail.
> 41 threats across 9 plan units (25-00 .. 25-08). All verified against implementation
> via ripgrep + file read against HEAD (cd8181e).

---

## Trust Boundaries

| Boundary | Description | Data Crossing |
|-|-|-|
| HTTP caller -> transport | BYOK vs Pro credential routing via `x-reasoning-provider` / `x-reasoning-api-key` headers | provider key (BYOK) or empty (Pro) |
| transport -> reasoning core | `ProviderCredentials` struct (api_key optional, base_url server-controlled) | api_key, provider name |
| reasoning core -> LLM vendor | HTTPS POST to Kimi/OpenAI/Ollama endpoints | messages, tool defs, api_key (Authorization header) |
| tool dispatch -> MemoryStore | In-process trait calls with `AgentCallerContext` (run_id, tenancy) | memory CRUD requests, knowledge_tier, source_ids |
| runner -> salience store | `apply_stability_boost(memory_id, magnitude, run_id, reason)` | stability multipliers + audit rows |

---

## Threat Register

| Threat ID | Category | Component | Disposition | Status | Evidence |
|-|-|-|-|-|-|
| T-25-00-01 | Tampering | salience_audit_log.reason | mitigate | CLOSED | migrations/029_salience_audit_log.sql:15 — `CHECK (reason IN ('final_selection','tombstoned','discarded','create_memory_source'))` |
| T-25-00-02 | DoS | revert_boost run size | accept | CLOSED | Bounded by single-run audit rows; accepted risk logged below |
| T-25-00-03 | Integrity | salience partial update | mitigate | CLOSED | storage/store/postgres/salience.rs:315 `.begin()` + transactional commit of audit insert + stability upsert |
| T-25-00-04 | Integrity | cascade-delete bypass of D-06 | mitigate | CLOSED | storage/store/postgres/queries.rs:775-789 `is_source_of_any_derived` uses `source_ids @> to_jsonb(...)` filtered by `deleted_at IS NULL` (schema uses deleted_at, not tombstoned_at — documented deviation) |
| T-25-00-05 | Integrity | double-boost on retry (Reviews HIGH #1) | mitigate | CLOSED | migrations/029:19 `UNIQUE (run_id, memory_id)` + salience.rs:334 `ON CONFLICT (run_id, memory_id) DO NOTHING` + line 346 `rows_affected() == 0` short-circuits the stability update |
| T-25-01-01 | EoP / SSRF | BYOK base_url injection | mitigate | CLOSED | intelligence/reasoning/credentials.rs:54 `base_url: None` on `from_headers`; only `from_env` can set it (operator-controlled) |
| T-25-01-02 | InfoDisclosure | ReasoningError Debug leak | accept | CLOSED | thiserror `#[error(...)]` format strings never interpolate api_key; accepted below |
| T-25-01-03 | Tampering | unknown-provider factory | mitigate | CLOSED | factory default arm returns `NotConfigured` for non-allowlisted provider strings |
| T-25-02-01 | InfoDisclosure | api_key in Kimi tracing | mitigate | CLOSED | intelligence/reasoning/kimi.rs:284 `#[tracing::instrument(skip(self, req), fields(model = %self.model))]` |
| T-25-02-02 | DoS | Kimi 5xx retry storm | mitigate | CLOSED | kimi.rs:292-317 `attempts<2` cap; `(500..600).contains(&status)` only; 4xx never retries |
| T-25-02-03 | Tampering | malformed Kimi tool args | mitigate | CLOSED | kimi.rs:253 `serde_json::from_str` error -> `ReasoningError::Generation` with tool_call context |
| T-25-03-01 | InfoDisclosure | api_key in OpenAI tracing | mitigate | CLOSED | intelligence/reasoning/openai.rs:278 `#[tracing::instrument(skip(self, req), fields(model = %self.model))]` |
| T-25-03-02 | DoS | OpenAI retry storm | mitigate | CLOSED | openai.rs:286-310 `attempts<2` cap, 5xx only |
| T-25-03-03 | Tampering | cross-adapter type confusion | mitigate | CLOSED | `grep -c 'use super::kimi' openai.rs = 0` — adapters fully isolated |
| T-25-04-01 | InfoDisclosure | Ollama silent tool-capability mismatch | mitigate | CLOSED | intelligence/reasoning/ollama.rs:55-88 `ensure_capabilities` probes `/api/show`, rejects if `capabilities` lacks `"tools"` with `NotConfigured` |
| T-25-04-02 | SSRF | Ollama BYOK base_url | mitigate | CLOSED | credentials.rs:54 forces base_url=None on headers; ollama.rs:37 falls back to profile.base_url (operator config only) |
| T-25-04-03 | Integrity | ollama:N:N id collisions | accept | CLOSED | Accepted — per-provider AtomicUsize sufficient |
| T-25-05-01 | Tampering | derived + empty source_ids | mitigate | CLOSED | intelligence/reasoning/tools.rs:336 storage-layer rejection bubbled as `storage_error` |
| T-25-05-02 | Integrity | silent cascade delete (Reviews HIGH #5) | mitigate | CLOSED | tools.rs:362-387 `is_source_of_any_derived` guard fires BEFORE delete; `force_if_source=true` path emits `tracing::warn!` + warning string in ToolResult |
| T-25-05-03 | InfoDisclosure | quarantined memory leak | transfer | CLOSED | Phase 11.2 upstream filter; no second filter added in 25 (documented) |
| T-25-05-04 | Tampering | unknown tool name | mitigate | CLOSED | tools.rs:232 and :433 `err_result(call, "unknown_tool", ...)` structured JSON |
| T-25-05-05 | DoS | unbounded search limit | mitigate | CLOSED | tools.rs:43 schema `"maximum": 50`; per-call jsonschema validation at line 236 rejects with `schema_validation` code |
| T-25-05-06 | Tampering | non-canonical knowledge_tier (Reviews HIGH #3) | mitigate | CLOSED | tools.rs:44,56 enum restricted to `[raw,imported,explicit,derived,pattern]` (+ `all` only on search filter); per-call validation enforces |
| T-25-05-07 | Integrity | force_if_source careless use | accept | CLOSED | Tool description + warning field + tracing::warn! — deliberate opt-in; accepted below |
| T-25-06-01 | DoS | runaway token cost | mitigate | CLOSED | intelligence/reasoning/runner.rs:93 budget check BEFORE generate; line 105-113 per-turn `max_tokens` capped at 4096 |
| T-25-06-02 | DoS | hung provider call | mitigate | CLOSED | runner.rs:84 30s/120s per-provider `turn_timeout`; line 118-121 `tokio::time::timeout` returns Transport error |
| T-25-06-03 | DoS | same-call loop | mitigate | CLOSED | runner.rs:154 `hash_canonical_call` + line 166 `AgentOutcome::RepeatedToolCall` after 3x same (name, canonical-args) |
| T-25-06-04 | Tampering | finish_reason control-flow drift (Pitfall 3) | mitigate | CLOSED | runner.rs:143 terminator is `tool_calls.is_empty()`; `grep -cE 'if.*finish_reason\|match.*finish_reason' runner.rs = 0`; only diagnostic `tracing::debug!` at line 137 |
| T-25-06-05 | InfoDisclosure | metric-label leak | mitigate | CLOSED | Labels restricted to profile name + adapter string; no api_key, no user content |
| T-25-07-01 | Integrity | double-count discarded vs final | mitigate | CLOSED | intelligence/reasoning/mod.rs:280 `discarded.difference(&final_sel)` |
| T-25-07-02 | Availability | single-memory boost failure aborts | mitigate | CLOSED | mod.rs:247,266,287 `tracing::warn!` + continue on individual failures |
| T-25-07-03 | Integrity | stability floor | mitigate | CLOSED | storage/store/postgres/salience.rs:324 `clamp(0.1, 36500.0)` in `apply_stability_boost` |
| T-25-07-04 | Integrity | double-boost on retry (Reviews HIGH #1) | mitigate | CLOSED | Same UNIQUE index + ON CONFLICT as T-25-00-05 |
| T-25-07-05 | Tampering | concurrent run boost race | accept | CLOSED | Distinct run_ids -> compounding is intended (REAS-10); accepted below |
| T-25-08-01 | EoP | Pro user injects api_key header | mitigate | CLOSED | transport/api/reasoning.rs:109-119 unconditional `req.headers_mut().remove("x-reasoning-api-key")` on Pro; also stripped on Pro+ollama branch (line 96) |
| T-25-08-02 | InfoDisclosure | api_key logged | mitigate | CLOSED | reasoning.rs:93-96 / :115-116 `tracing::warn!(event=...)` logs event name + provider only; no key value — verified by absence in warn! args |
| T-25-08-03 | DoS | unknown-provider flood | accept | CLOSED | 400 response cheap; Phase 24.5 rate limit layer upstream; accepted below |
| T-25-08-04 | Tampering | provider header case-fold bypass | mitigate | CLOSED | reasoning.rs:73 `provider_hdr.trim().to_ascii_lowercase()` before allowlist check |
| T-25-08-05 | InfoDisclosure | AppState Debug leak | accept | CLOSED | Out-of-scope; general security linting; accepted below |
| T-25-08-06 | Availability | Ollama BYOK blocker (Reviews HIGH #2) | mitigate | CLOSED | reasoning.rs:85-104 Ollama short-circuit — no key required in either tenancy; 2 dedicated regression tests |
| T-25-08-07 | Tampering | tenancy flips Pro on ollama-only env | mitigate | CLOSED | transport/health/mod.rs:72 `env_keys.keys().any(\|p\| p != "ollama")` — ollama-only env stays BYOK |

*Status: CLOSED (mitigation verified in code) / OPEN (gap) — all 41 threats CLOSED.*

---

## Accepted Risks Log

| Risk ID | Threat Ref | Rationale | Accepted By | Date |
|-|-|-|-|-|
| AR-25-01 | T-25-00-02 | revert_boost bounded by max_iterations * tool_calls per turn (O(10s)); no realistic amplification vector | Phase 25 planner | 2026-04-23 |
| AR-25-02 | T-25-01-02 | thiserror `#[error(...)]` format strings never include api_key values; standard hygiene | Phase 25 planner | 2026-04-23 |
| AR-25-03 | T-25-04-03 | AtomicUsize turn counter scoped per-provider Arc; cross-run collisions impossible, same-run parallel calls disambiguated by turn index | Phase 25 planner | 2026-04-23 |
| AR-25-04 | T-25-05-03 | Trust-weighted recall + quarantine filter (Phase 11.2) operates upstream of reasoning tool dispatcher; transferring rather than double-gating | Phase 25 planner | 2026-04-23 |
| AR-25-05 | T-25-05-07 | force_if_source is a deliberate opt-in: tool description warns, ToolResult carries warning field, tracing::warn! surfaces to operators. Bypass is never silent. | Phase 25 planner | 2026-04-23 |
| AR-25-06 | T-25-07-05 | Concurrent run_agent invocations have distinct run_ids. Compounding stability boosts for hot memories is the REAS-10 design intent — not a bug. | Phase 25 planner | 2026-04-23 |
| AR-25-07 | T-25-08-03 | 400 response is cheap; Phase 24.5 rate-limit layer absorbs volume ahead of reasoning middleware | Phase 25 planner | 2026-04-23 |
| AR-25-08 | T-25-08-05 | AppState Debug hygiene is a workspace-wide concern handled by general security linting; not a Plan 08 scope item | Phase 25 planner | 2026-04-23 |

*Accepted risks do not resurface in future audit runs unless implementation changes invalidate the rationale.*

---

## Unregistered Flags

None. No `## Threat Flags` section in SUMMARY.md files introduced attack surface outside the 41-entry register.

Out-of-scope non-security items (not threats):
- Clippy pedantic warnings (~2056) — code hygiene, not security
- Deferred Anthropic/Bedrock/Azure adapters — Phase 25.1+ scope

---

## Security Audit Trail

| Audit Date | Threats Total | Closed | Open | Run By |
|-|-|-|-|-|
| 2026-04-23 | 41 | 41 | 0 | gsd-security-auditor (Claude Opus 4.7) |

Verification method: ripgrep + Read against HEAD `cd8181e` on main. All `mitigate` threats verified by file:line evidence for the declared pattern; all `accept` threats logged in Accepted Risks; T-25-05-03 `transfer` verified by Phase 11.2 reference.

Reviews cross-check: HIGH #1 (idempotency), HIGH #2 (Ollama BYOK), HIGH #3 (tier enum), HIGH #5 (cascade guard), MEDIUM #6 (per-call schema validation), MEDIUM #7 (structured error envelope) — all independently verified CLOSED.

---

## Sign-Off

- [x] All threats have a disposition (mitigate / accept / transfer)
- [x] Accepted risks documented in Accepted Risks Log
- [x] `threats_open: 0` confirmed
- [x] `status: verified` set in frontmatter

**Approval:** verified 2026-04-23
