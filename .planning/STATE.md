---
gsd_state_version: 1.0
milestone: v1.0
milestone_name: milestone
status: unknown
stopped_at: Phase 25 Plan 05 COMPLETE — 6 memory tools + dispatch_tool + add_annotation shipped; plans 06/07/08 unblocked
last_updated: "2026-04-22T06:55:30.375Z"
progress:
  total_phases: 67
  completed_phases: 41
  total_plans: 166
  completed_plans: 135
  percent: 81
---

# Project State

## Current Phase

Phase 25 (Reasoning Agent) — Plans 00 + 01 + 02 + 03 + 04 + 05 COMPLETE. Primitives, trait, all 3 adapters (Kimi/OpenAI/Ollama), and the 6-tool palette + dispatch_tool shipped. Plans 06 (runner loop) + 07 (salience hook) + 08 (BYOK wiring) ready to land. Phase 24.75 Plan 05 (benchmark re-run) still remaining in parallel.

## Active Context

- Phase 24 COMPLETE: 4 plans, 12 tests, gap closure (tier_filter threading), human-approved
- Phase 24.5 Plan 00 COMPLETE: 22 integration + 4 unit `#[ignore]` stubs; tool-count bumped 16→18 (intentional red until 24.5-04); MemoryBuilder.reply_to_id() added
- Phase 24.5 Plan 01 COMPLETE: migration 027 + nullable reply_to_id column + idx_memories_reply_to_id partial index; StoreOutcome enum; store_with_outcome canonical path; store() now provided default; INSERT/SELECT threaded; test_reply_to_id_migration green; 21 other ingest stubs still ignored
- Phase 24.5 Plan 02 COMPLETE: IngestConfig { api_key, max_batch_size=100, max_content_size=32768 } + rate_limit.ingest_rps=50; memcp::transport::boot_safety::check_ingest_auth_safety (IpAddr::is_loopback + 'localhost'); daemon gate before axum::serve; test_boot_fails_non_loopback_no_key GREEN with 6 assertions; threat T-24.5-01 mitigated
- Phase 24.5 Plan 03 COMPLETE: AuthState + require_api_key middleware; make_idempotency_key (SHA-256 length-prefixed); IngestMessage/Request/Result/Summary DTOs; shared pipeline/auto_store/shared.rs::process_ingest_message; /v1/ingest handler + run_ingest_batch; route composition .layer(rate_limit).layer(auth) with auth OUTERMOST; AppState extended (auth, content_filter, summarization_provider, extract_sender); 14 HTTP tests + 3 unit tests green; auto-store refactored onto shared helper with zero regression; threats T-24.5-02/03/04/05 mitigated
- Phase 24.5 Plan 04 COMPLETE: parse_ingest_stream auto-detect helper; run_ingest_batch refactored into AppState-free run_ingest_batch_with_ctx shared by HTTP/MCP/CLI; 2 MCP tools (ingest_messages batch + ingest_message single) registered; MemoryService gains summarization_provider/embed_sender/extract_sender/ingest_config/user_birth_year setters; memcp ingest CLI subcommand with --file/--message/--stdin (auto-detect when piped); cmd_ingest_from_reader testable seam; 22/22 ingest tests green; test_tool_discovery (16→18) now green; INGEST-06 delivered in full
- Phase 24.75 Plan 00 COMPLETE: A1 probe (A1-UNDECIDABLE-EMPTY — dev DB has 0 chunk rows; source-read confirms A1 holds by construction); 3 RED test scaffolds registered in tests/chunk_removal_test.rs (all `#[ignore]`d with pending-plan markers)
- Phase 24.75 Plan 01 COMPLETE: migrations/028_drop_chunks.sql (DDL-only, 10 lines); migrate_028_collapse_chunks binary (438 lines, local-embed feature) — Pitfall 5 dimension guard + Pitfall 6 per-parent short transactions + A1 length-guardrail before any wipe; data/migration_028_report.jsonl audit trail; dry-run on dev DB prints "no chunk rows — no-op"; 4 unit tests + 1 integration test green; CHUNK-02 delivered. test_migration_028_refuses_unreassembled + test_columns_dropped still `#[ignore]`d (Plan 03 owners)
- Phase 24.75 Plan 04 COMPLETE: compute_memory_span shared helper in transport/api/memory_span.rs — MCP + HTTP + CLI all delegate to one code path (byte-identical output for same inputs). MCP get_memory_span tool registered (tool-count 18→19); POST /v1/memory/span behind rate_limit; `memcp memory-span --id --topic` CLI subcommand. Topic-embedding cache bounded to 100 entries on AppState + MemoryService. Byte offsets computed via sentence-anchor back into memory.content, returned content is the parent substring verbatim. Threats T-24.75-04-01/02/03/05 mitigated (topic length cap, span count cap, uniform not-found, no topic logging). 3 Wave-0 scaffolds flipped green (test_topic_ranked_span, test_memory_span_offsets_valid, test_memory_span_http); CHUNK-04 delivered.
- Phase 25 Plan 00 COMPLETE: migration 029_salience_audit_log.sql (UNIQUE (run_id, memory_id) + CHECK on 4-value reason enum) applied to dev DB; apply_stability_boost (transactional INSERT ... ON CONFLICT DO NOTHING + rows_affected()==0 short-circuit → idempotent per (run_id, memory_id), Reviews HIGH #1 closed) and revert_boost (per-run rollback) in postgres/salience.rs; is_source_of_any_derived moved to MemoryStore trait (Ok(false) default) with PostgresMemoryStore override in queries.rs so &dyn callers hit the real EXISTS query; jsonschema 0.46 + wiremock 0.6 deps added; 27 #[ignore]'d RED scaffolds across 11 reasoning_*.rs files (5 salience incl. HIGH #1 double-invoke, 5 tool_dispatch incl. HIGH #3/#5 + MEDIUM #6, 4 byok_boundary incl. HIGH #2 ollama-no-key). REAS-10 primitives ready.
- Phase 25 Plan 01 COMPLETE: intelligence::reasoning module with async ReasoningProvider trait (generate + model_name), 9 unified wire types (Tool/ToolCall/ToolResult/Message/TokenUsage/ReasoningRequest/ReasoningResponse/AgentOutcome/AgentCallerContext), ProviderCredentials { api_key, base_url } with from_env (MEMCP_REASONING__<P>_API_KEY) + from_headers (x-reasoning-api-key; base_url hard-coded None — SSRF T-25-01-01) + require_api_key; create_reasoning_provider factory matches on kimi/openai/ollama with NotConfigured default arm; 3 stub adapter modules (kimi/openai/ollama) return NotConfigured from new() so plans 02-04 can diff in cleanly. ReasoningConfig + ProfileConfig appended to config.rs with seed dreaming (kimi+kimi-k2.5+12 iter+32k budget+0.3 temp) and retrieval (kimi+kimi-latest+6 iter+8k budget+0.2 temp); resolve(name) falls back to default_profile="retrieval" on empty name; wired into top-level Config via #[serde(default)]. From<ReasoningError> for MemcpError via Internal variant. Wave 0 reasoning_trait_test::trait_compiles flipped RED → GREEN. 4 tests green (1 integration + 3 lib config). REAS-01 + REAS-09 delivered.
- Phase 25 Plan 05 COMPLETE: intelligence/reasoning/tools.rs — memory_tools() returns 6 Tool defs (search_memories/create_memory/update_memory/delete_memory/annotate_memory/select_final_memories) with canonical Phase 24 knowledge_tier enum [raw,imported,explicit,derived,pattern] (HIGH #3); dispatch_tool runs jsonschema::validator_for(&tool.parameters).validate(&call.arguments) BEFORE serde_json::from_value (MEDIUM #6); ALL error ToolResults are structured JSON {"error","code"} via single err_result() helper with distinct codes schema_validation/bad_args/storage_error/unknown_tool/cascade_delete_forbidden/bad_tool_schema (MEDIUM #7); delete_memory fires MemoryStore::is_source_of_any_derived BEFORE delete with force_if_source=true escape hatch emitting tracing::warn! + warning field in ToolResult (HIGH #5). MemoryStore::add_annotation trait method added (default Err(Internal)) with Postgres inherent impl via jsonb_set(metadata,'{annotations}',coalesce(…)||to_jsonb($2::text)) + updated_at=NOW() bump (LOW #11 comment inline); trait forwarder in queries.rs routes &dyn callers to inherent. search_memories delegates to MemoryStore::list + in-memory substring filter as MVP (plan referenced recall::recall free function that doesn't exist — real RecallEngine::recall needs an embedding; hybrid_search via dispatcher deferred to Phase 27). 4 lib tests (tool_schema_tests) + 1 integration smoke + 8 DB-gated integration tests all GREEN. REAS-06 delivered; Reviews HIGH #3, HIGH #5, MEDIUM #6, MEDIUM #7 closed.
- Phases 24.5-27 on ROADMAP (Universal Ingestion, Reasoning Agent, Dreaming Worker, Agentic Retrieval)
- Pricing decided: Option A -- Pro $25-35/mo includes reasoning, BYOK $10-15/mo

## Next Steps

1. Plan 24.75-05: re-run LongMemEval + LoCoMo benchmarks post-chunk-removal. Accept ≤5% recall drop per D-05; document findings. If >5%, investigate but do not revert (precision path is Phase 27 + Phase 29, not reviving chunks).
2. Open a workspace-wide clippy sweep follow-up (2056 pre-existing pedantic warnings in load_test + validation, blocking `cargo clippy --all-targets -- -D warnings`).
3. After Phase 24.75 wraps: coordinate downstream update — Phase 27 ARET-02 should add `get_memory_span` to the retrieval specialist's tool palette.

## Project Reference

See: .planning/PROJECT.md (updated 2026-04-17)

**Core value:** Persistent memory for AI agents via MCP + CLI
**Current focus:** Phase 25 — reasoning-agent

## Session Continuity

Last session: 2026-04-22T06:55:30.347Z
Stopped at: Phase 25 Plan 05 COMPLETE — 6 memory tools + dispatch_tool + add_annotation shipped; plans 06/07/08 unblocked
Resume file: None

## Recent Decisions

- 24.5-00: hold `.reply_to_id` on MemoryBuilder but defer CreateMemory threading until 24.5-01 (field doesn't exist yet)
- 24.5-00: collapse tool-count assertion to single line so plan grep `assert_eq!(tools.len(), 18` matches verbatim
- 24.5-00: pre-register 18-tool name assertions before MCP tool registration — test goes RED intentionally until 24.5-04 (RESEARCH pitfall 6)
- 24.5-01: keep store() as provided trait default (not removed) so existing call sites remain byte-identical; store_with_outcome is the canonical path
- 24.5-01: StoreOutcome enum pattern for dedup signalling — public in storage::store, imported directly by transport layer (no wrapper needed)
- 24.5-01: thread reply_to_id only through queries.rs SELECTs this plan; row_to_memory uses unwrap_or(None) so SELECTs in embedding.rs/extraction.rs/salience.rs silently yield None until a downstream plan needs them
- 24.5-02: boot-safety helper lives in memcp-core (transport/boot_safety.rs), not binary crate — integration tests call it directly; binary wraps with eprintln + exit(1)
- 24.5-02: daemon gate placed INSIDE the `if config.health.enabled` branch — non-loopback + no-key is safe when HTTP is disabled, dangerous when enabled; matches plan guidance "keep it in the serve/daemon subcommand path"
- 24.5-02: IPv6-aware port stripping — bare '::1' is NOT rsplit on final ':' because it parses cleanly; bracketed '[::1]:8080' strips via '[' / ']' markers
- 24.5-02: 6 inline unit tests + 1 integration test (6 assertions) for the boot-safety helper — 12 distinct scenarios because pitfall 5 is easy to regress
- 24.5-03: shared pipeline helper (`process_ingest_message`) stages 1-9 live in pipeline/auto_store/shared.rs; chunking stays in auto-store worker (D-10 update 2026-04-18 explicitly excluded chunking from ingest)
- 24.5-03: AppState carries pipeline deps (content_filter, summarization_provider, extract_sender) as `Option<Arc<dyn Trait>>` slots — mirrors existing redaction_engine pattern
- 24.5-03: daemon reorders content_filter + summarization construction BEFORE health-server spawn so AppState carries them at router-build time (D-10 pipeline parity for /v1/ingest)
- 24.5-03: `prev_id` advances on BOTH Stored and Deduplicated outcomes; filter/error leave it unchanged — a dedup reply should still chain to its predecessor
- 24.5-03: caller `reply_to_id` evaluated BEFORE `prev_id` (`msg.reply_to_id.or_else(|| prev_id.clone())`) — D-18 takes precedence over D-17
- 24.5-03: rate-limit-disabled branch still applies the auth layer (passthrough when api_key=None) for defense-in-depth on dev misconfigurations
- 24.5-03: `extract_sender` on AppState stays None in daemon path — extraction pipeline is built after health server; auto-store still gets it via its own context. Future plan can rewire if /v1/ingest extraction parity is required
- 24.5-04: run_ingest_batch_with_ctx is the canonical shared entry — HTTP/MCP/CLI all delegate to one per-message loop instead of three parallel implementations (removes drift risk)
- 24.5-04: MCP ingest params (IngestMessageParams, IngestMessagesParams, IngestSingleMessageParams) are separate structs from the HTTP DTOs (JsonSchema required by rmcp; bloats HTTP schema if merged). `From<IngestMessageParams> for IngestMessage` bridges the two
- 24.5-04: IngestSingleMessageParams flattens message fields at top level (not `{message: {...}, ...}`) — matches how agents call store_memory one flat object
- 24.5-04: MemoryService gains 5 optional setter fields instead of expanding the 12-arg `::new()` signature — preserves construction-site stability across binary + load_test.rs
- 24.5-04: cmd_ingest splits into binary-entry / testable-seam / internal helper. `cmd_ingest_from_reader<R: Read>` is the seam CLI tests use with `Cursor` — exact same code path as piped stdin without touching real stdin
- 24.5-04: CLI path passes all-None optional deps (redaction, content_filter, summarization, embed_sender, extract_sender) — DB-direct like cmd_store. Users who need the full D-10 pipeline should call HTTP daemon
- 24.5-04: committed Task 1 + Task 2 as a single atomic commit (3adb8ae) — shared file touches (main.rs, ingest_test.rs) and inter-task refactor (run_ingest_batch_with_ctx) make per-task splits risk breaking intermediate state
- 24.75-01: two-step migration 028 (Rust orchestrator for async re-embed + DDL-only SQL) — Research R-1 split, cannot run embedding I/O inside `sqlx::migrate!`
- 24.75-01: A1-UNDECIDABLE-EMPTY path — trust parent.content by construction (auto_store inserts full content before fan-out); add runtime length(parent.content) > 0 guardrail before wiping chunks (A1-PROBE belt-and-braces)
- 24.75-01: detect_and_reassemble kept as A1-REFUTED fallback (header-stripped concat in chunk_index order); zero cost on empty dev DB, safety net for any other DB
- 24.75-01: two separate pool/store entities in the binary — direct PgPool for write transactions, PostgresMemoryStore for read helpers; simpler than adding a pool-sharing constructor for one-shot binary
- 24.75-01: integration test re-declares detect_and_reassemble as a lockstep shim (integration-test crates cannot import from src/bin/*.rs); binary's 4 inline unit tests are canonical coverage
- 24.75-04: compute_memory_span uses splitter::split_sentences directly (NOT chunk_content) because chunk_content prefixes a `[From: topic] part N/M\n` context header that would break the `memory.content[start..end] == returned.content` byte invariant
- 24.75-04: return memory.content[start..end] VERBATIM (not the joined-sentences string) — guarantees byte-accurate offsets even when unicode_sentences consumes inter-sentence whitespace; offsets anchor via first + last sentence find into parent content
- 24.75-04: HTTP /v1/memory/span sits behind rate_limit only, no auth — mirrors /v1/search (A4); topic queries are read-only and cheap enough to share the search rate bucket
- 24.75-04: topic-embedding cache is bounded HashMap<String, Vec<f32>> cap=100, drop-arbitrary-entry on overflow — RESEARCH Don't-Hand-Roll simple path is acceptable for v1; span-embedding cache deferred until real usage demands (Phase 29 may obsolete it)
- 24.75-04: MemoryService gains topic_embedding_cache field + setter so a daemon can share one Arc with AppState when hosting both HTTP + MCP — stdio-only `memcp serve` gets its own default cache
- 24.75-04: MCP + HTTP both map MemcpError::NotFound to a uniform "memory not found" error — T-24.75-04-03 disposition prevents scope-exclusion disclosure
- 24.75-04: tracing span emits tool + memory_id only; topic is NEVER logged — T-24.75-04-05 disposition avoids content leakage via logs
- 24.75-04: 8 AppState struct-literal call sites all required the new topic_embedding_cache field (daemon.rs + 6 test helpers + load_test.rs × 2) — plan called out api_test.rs only; the rest were Rule-3 auto-fixes
- 25-00: Salience audit idempotency uses INSERT ... ON CONFLICT DO NOTHING + rows_affected()==0 short-circuit inside a tx — NOT a SELECT-first guard. Closes the race window Gemini flagged (HIGH #1) without a separate lock.
- 25-00: is_source_of_any_derived lives on the MemoryStore trait (default Ok(false)) with a PostgresMemoryStore override — plan 05's &dyn MemoryStore dispatch needs trait-resolvable visibility, inherent methods would silently fall back to the permissive default.
- 25-00: Plan referenced `tombstoned_at`; memcp schema uses `deleted_at` (Rule 1 auto-fix). The EXISTS query now reads `deleted_at IS NULL`; docstring documents the deviation so downstream plans aren't misled.
- 25-00: RED scaffolds use `unimplemented!()` body, not `assert!(false, ...)` — flipping `#[ignore]` off without a real impl panics loudly (no false-green risk).
- 25-01: ToolCall.arguments is always a parsed serde_json::Value (not String) — adapters normalize stringified JSON at translate_out boundary per RESEARCH Pitfall 1; dispatcher never re-parses.
- 25-01: Task order reversed (config committed before trait) because intelligence::reasoning imports crate::config::ProfileConfig — plan listed them in reverse logical order; Rule 3 blocker fix.
- 25-01: Test import uses `memcp::` (lib name), not `memcp_core::` (package name); plan specified the wrong prefix — matches existing test convention across store_test/stress_test/import_test.
- 25-01: base_url NEVER populated on the BYOK path (from_headers); adapter defaults always win. Pro env reads permit base_url override but only under operator control (SSRF T-25-01-01 mitigation).
- 25-01: ReasoningError → MemcpError via the Internal variant (sanitize_message keeps adapter strings safe) rather than minting a new top-level variant — keeps error surface stable.
- 25-05: dispatch_tool runs jsonschema::validator_for(&tool.parameters).validate(&call.arguments) BEFORE serde_json::from_value — distinguishes schema_validation errors (LLM malformed) from bad_args (type mismatch) in the agent's feedback loop (MEDIUM #6).
- 25-05: Structured JSON error envelope via single err_result(call, code, msg) helper — every error path routes through one place so agents always see {"error","code"} (MEDIUM #7 enforced at call-site level).
- 25-05: delete_memory force_if_source=true emits tracing::warn! AND a warning field in the ToolResult body — escape hatch observable in operator logs AND agent context (HIGH #5).
- 25-05: search_memories dispatches via MemoryStore::list + in-memory substring filter as MVP — plan's referenced recall::recall(store, query, limit, tier) free function doesn't exist; real RecallEngine::recall takes embedding bound to concrete PostgresMemoryStore. Hybrid search via dispatcher deferred to Phase 27 agentic retrieval.
- 25-05: add_annotation UPDATE binds id with plain $1 (no ::uuid cast) — matches queries.rs delete/update/touch pattern; the ::uuid cast produced silent sanitized "Database operation failed" on live dev DB until removed (Rule 1 auto-fix during Task 2).
- 25-05: create_memory inserts source_ids into AgentCallerContext.final_selection alongside the stored id — provenance nodes flow through the REAS-10 stability boost, not just the terminal memory.
- 25-05: select_final_memories removes ids from read_but_discarded so a final-selected id never double-accounts as discarded-but-selected.
- 25-05: `CreateMemory` has no `Default` impl — dispatcher + tests use explicit field-by-field construction via `build_create_memory`/`sample_create` helpers.
- 25-05: StoreOutcome variants are `Created(Memory) | Deduplicated(Memory)` (not `Stored{..}` as plan assumed) — dispatcher uses `outcome.memory().id.clone()` which works for both.

**Planned Phase:** 25 (Reasoning Agent) — 9 plans — 2026-04-20T23:07:08.506Z
