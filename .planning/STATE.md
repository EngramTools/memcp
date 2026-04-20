---
gsd_state_version: 1.0
milestone: v1.0
milestone_name: milestone
status: unknown
stopped_at: 24.75-04 complete — get_memory_span across MCP + HTTP + CLI
last_updated: "2026-04-20T02:48:37.854Z"
progress:
  total_phases: 67
  completed_phases: 41
  total_plans: 157
  completed_plans: 130
  percent: 83
---

# Project State

## Current Phase

Phase 24.75 (Chunk-Semantics Rethink) -- Plans 00-04 COMPLETE. Plan 05 (benchmark re-run on LongMemEval + LoCoMo post-chunk-removal) remaining.

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
- Phases 24.5-27 on ROADMAP (Universal Ingestion, Reasoning Agent, Dreaming Worker, Agentic Retrieval)
- Pricing decided: Option A -- Pro $25-35/mo includes reasoning, BYOK $10-15/mo

## Next Steps

1. Plan 24.75-05: re-run LongMemEval + LoCoMo benchmarks post-chunk-removal. Accept ≤5% recall drop per D-05; document findings. If >5%, investigate but do not revert (precision path is Phase 27 + Phase 29, not reviving chunks).
2. Open a workspace-wide clippy sweep follow-up (2056 pre-existing pedantic warnings in load_test + validation, blocking `cargo clippy --all-targets -- -D warnings`).
3. After Phase 24.75 wraps: coordinate downstream update — Phase 27 ARET-02 should add `get_memory_span` to the retrieval specialist's tool palette.

## Project Reference

See: .planning/PROJECT.md (updated 2026-04-17)

**Core value:** Persistent memory for AI agents via MCP + CLI
**Current focus:** Phase 24.75 — chunk-semantics-rethink

## Session Continuity

Last session: 2026-04-20T02:50:00.000Z
Stopped at: 24.75-04 complete — get_memory_span across MCP + HTTP + CLI (CHUNK-04 delivered)
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
