---
gsd_state_version: 1.0
milestone: v1.0
milestone_name: milestone
status: unknown
last_updated: "2026-02-28T20:10:00.000Z"
progress:
  total_phases: 36
  completed_phases: 13
  total_plans: 78
  completed_plans: 47
---

# Project State

## Current Phase
Phase 08.8-plugin-support-primitives — Plan 05 complete (5/5 plans DONE)

## Active Context
- Last completed: Phase 08.8-05 recall output improvements (2026-02-28)
- cmd_recall gains --first flag: injects current_datetime, preamble text, and recalled memories
- Regular recall (first=false) returns {session_id, count, memories} — no preamble overhead
- truncate_content helper trims to config.recall.truncation_chars (default 200) with ... indicator
- build_related_hint builds "memcp search --tags ..." ready-made command from shared tags
- RelatedContext struct in postgres.rs; get_related_context batch method counts tag-sharing memories
- Trivial tags (auto-stored, summarized, merged, stale, category:*) filtered from hints
- DEFAULT_PREAMBLE const; overridable via config.recall.preamble_override
- related_count/hint only included when related_count > 0 (no empty noise)
- 68 tests passing total (42 lib + 26 integration)
- Last session: 2026-02-28
- Stopped at: Completed 08.8-04-PLAN.md
- Next: Phase 08.8 Plan 05 — related context + recall enhancements

## Accumulated Context

### Phase 08.8 Decisions
- Phase 08.8-05: RelatedContext struct placed in postgres.rs — store layer owns the query, CLI layer owns formatting
- Phase 08.8-05: Two-phase related context: batch tag fetch for all IDs, then per-memory count query — avoids N+1 on the outer loop
- Phase 08.8-05: SKIP_TAGS filters auto-stored/summarized/merged/stale/category:* — prevents useless hints pointing agents to system tags
- Phase 08.8-05: related_count/hint omitted when related_count == 0 — no empty noise in output
- Phase 08.8-05: current_datetime + preamble injected at output stage (not RecallResult) — first=true is CLI-only concern, RecallEngine stays clean
- Phase 08.8-04: source_line in ids.jsonl emission uses 0 as placeholder — WatchEvent carries line content not byte offset; plugin consumers correlate via memory_id + content_preview
- Phase 08.8-04: Chunks inherit parent event_time/event_time_precision instead of re-extracting from chunk content — chunks are fragments, parent has the full temporal reference
- Phase 08.8-04: run_temporal_worker _shutdown_tx held in local daemon scope — broadcast channel keeps worker alive for daemon lifetime; worker exits on next tick when sender drops (natural cleanup)
- Phase 08.8-04: Temporal LLM worker queries extraction_status='complete' rows only — avoids racing with ongoing extraction pipeline processing
- Phase 08.8-01: event_time_precision uses TEXT CHECK constraint (not Postgres ENUM) — easier to extend without ALTER TYPE per CONTEXT.md pitfall guidance
- Phase 08.8-01: workspace partial index WHERE workspace IS NOT NULL — excludes global (NULL) memories from index for efficiency
- Phase 08.8-01: TemporalConfig.openai_base_url is Option<String> — None means use provider default, distinguishes absent-from-config vs explicit override
- Phase 08.8-01: All 10 CreateMemory struct literals updated with new fields=None — event_time/workspace population reserved for Plans 03/04
- Phase 08.8-01: RecallConfig extended with truncation_chars (200), preamble_override (None), related_context_enabled (true) — config-only for now, wire-up in Plan 05
- Phase 08.8-02: annotate_logic() extracted as shared pub async fn in cli.rs — CLI and MCP both call it, no duplication
- Phase 08.8-02: Memory.tags is Option<serde_json::Value> (JSONB) not Vec<String> — parsed via as_array().filter_map(as_str) chain
- Phase 08.8-02: UpdateMemory struct has only 4 fields (content, type_hint, source, tags) — no actor/actor_type/audience; plan interfaces section was aspirational
- Phase 08.8-03: Temporal regex priority order: month-year > year > decade > relative-age > relative-month > relative-day (most-specific-first, first match wins)
- Phase 08.8-03: Decade regex captures 2-digit prefix ("90" from "90s"), year = 1900+prefix or 2000+prefix; picks most-recent-past decade ≤ now.year
- Phase 08.8-03: Workspace filter is application-level post-filter on hybrid_search (OR workspace IS NULL) — consistent with existing source/audience post-filter pattern, avoids SQL complexity
- Phase 08.8-03: recall_candidates workspace filter uses dynamic SQL format!() to append $5 clause — avoids duplicating static SQL for extraction-on and extraction-off tiers
- Phase 08.8-03: RecallEngine.recall() gains workspace param at call-site (not stored on struct) — stateless control, simpler API
- Phase 08.8-03: format_memory_json compact branch conditionally inserts non-null fields (event_time/event_time_precision/workspace) to save tokens; verbose always includes all fields
- Phase 08.8-02: annotate_memory added to tool_router_with_meta() allowed_callers (direct + code_execution_20260120) — non-destructive enrichment mutation

### Phase 08.7 Decisions
- Phase 08.7: Plan 02 — Tasks 1-3 pre-implemented in 08.7-01; Task 4 wires EmbeddingRouter into auto-store with Option<Arc<EmbeddingRouter>> — None in serve mode, Some(router) in daemon mode; chunks inherit parent tier
- Phase 08.7: Plan 03 — promotion sweep pre-implemented in 08.7-01 (0f6d57b) as forward-work; plan 03 is verification + documentation only
- Phase 08.7: Sweep skips with skipped_reason="No promotion candidates found" when candidates list is empty (not an error)
- Phase 08.7: Old fast-tier embedding deactivated (is_current=false) before new quality-tier embedding inserted — prevents duplicate current embeddings
- Phase 08.7: Multi-model embeddings with tiered config — single-tier (empty tiers HashMap) = backward compatible legacy mode
- Phase 08.7: EmbeddingTierConfig with provider, model, openai_api_key, dimension, routing (RoutingConfig), promotion (PromotionConfig)
- Phase 08.7: RoutingConfig: AND logic — all specified conditions (min_stability, type_hints, min_content_length) must be met
- Phase 08.7: PromotionConfig: min_reinforcements=3, min_stability=0.8, sweep_interval_minutes=60, batch_cap=15
- Phase 08.7: EmbeddingRouter wraps HashMap<String, TierEntry> keyed by tier name, implements EmbeddingProvider (delegates to default tier)
- Phase 08.7: EmbeddingPipeline.new() takes Arc<EmbeddingRouter>; new_single() wraps a single provider for backward compat
- Phase 08.7: EmbeddingJob gains `tier: String` field (default "fast"), EmbeddingCompletion gains `tier: String`
- Phase 08.7: Migration 017: `ALTER TABLE memory_embeddings ADD COLUMN tier TEXT NOT NULL DEFAULT 'fast'` + per-tier indexes
- Phase 08.7: ensure_hnsw_index_for_tier creates partial indexes `WHERE tier = '{name}'` with correct dimension cast
- Phase 08.7: Promotion sweep: daemon spawns periodic worker if router.is_multi_model() && quality tier has promotion config
- Phase 08.7: run_promotion_sweep: fetch candidates via get_promotion_candidates, embed with quality provider, deactivate old + insert new
- Phase 08.7: embed_query_all_tiers: lazy quality optimization — skips non-default tiers with zero embeddings in corpus
- Phase 08.7: hybrid_search_multi_tier: BM25 + symbolic once (text-based), vector search per tier via search_vector_for_tier, RRF merge with best-rank dedup
- Phase 08.7: build_embedding_router in daemon.rs: legacy mode wraps create_embedding_provider in single-tier router; multi-tier creates per-tier providers via create_tier_provider
- Phase 08.7: CLI and MCP serve use single-tier search (backward compat); multi-tier search available via hybrid_search_multi_tier
- Phase 08.7: Plan 04 — embed_multi IPC type returns HashMap<tier, Vec<f32>> over existing socket; start_embed_listener accepts Option<(Arc<EmbeddingRouter>, Arc<PostgresMemoryStore>)>
- Phase 08.7: CLI detects multi-model via config.embedding.is_multi_model() — no runtime discovery needed
- Phase 08.7: recall_candidates_multi_tier merges by best relevance score; single tier delegates to recall_candidates directly
- Phase 08.7: Single-model daemon returns error for embed_multi — CLI degrades to text-only (fail-open)

### Phase 08.6 Decisions
- Phase 08.6: Algorithmic-first curation — no LLM required by default, CurationConfig.llm_provider=None uses AlgorithmicCurator
- Phase 08.6: CurationProvider trait (async_trait) with review_cluster, synthesize_merge, model_name
- Phase 08.6: AlgorithmicCurator: stale = low salience (<threshold) + old (>age_days) + unreinforced; strengthen = reinforcement_count>=3 + stability<5.0; merge = all members low salience
- Phase 08.6: Ollama + OpenAI LLM curation providers reuse shared format_cluster/parse_review_response helpers
- Phase 08.6: Greedy clustering via pgvector embedding similarity (threshold 0.85, max group size 5)
- Phase 08.6: Per-run caps: 20 merges, 50 flags, 50 strengthens per curation pass
- Phase 08.6: Merge creates new memory (type_hint="curated", tag "merged", source="curation"), soft-deletes originals, stability=max(sources)
- Phase 08.6: FlagStale adds "stale" tag + demotes stability to config.stale_stability_target (0.1)
- Phase 08.6: Strengthen calls reinforce_salience("good") + adds "curated:strengthened" tag
- Phase 08.6: Per-run undo: undo_curation_run restores originals, deletes merged, reverts stability, removes tags
- Phase 08.6: Windowed scan uses get_last_successful_curation_time as window start, excludes recently-curated (type_hint='curated')
- Phase 08.6: Migration 016_curation.sql — curation_runs + curation_actions tables with full action tracking
- Phase 08.6: CLI: `memcp curation run [--propose]`, `memcp curation log [--limit N]`, `memcp curation undo <run_id>`

### Phase 08.5 Decisions
- Phase 08.5-01: Sync store uses tokio::sync::oneshot channel on EmbeddingJob — worker sends EmbeddingCompletion with status/dimension; CLI path polls embedding_status every 200ms
- Phase 08.5-01: StoreConfig.sync_timeout_secs (default 5) configures wait timeout
- Phase 08.5-01: reembed_on_tag_change (default false) — content-only triggers re-embed by default; tag changes skip re-embed unless explicitly opted in
- Phase 08.5-01/02: ResourceLimitsConfig with warn_percent=80, hard_cap_percent=110, auto_gc=false, auto_gc_cooldown_mins=15 — tiered capacity check replaces old binary reject
- Phase 08.5-01/02: Auto-GC uses Instant-based cooldown guard in Arc<Mutex<Option<Instant>>> — fire-and-forget with 15-minute minimum between runs
- Phase 08.5-03: LlmCategoryClassifier sends taxonomy prompt to Ollama/OpenAI, parses single-word response — 3-second timeout with heuristic fallback (fail-open)
- Phase 08.5-03: CategoryResult cached via Arc<Mutex<Option<CategoryResult>>> on CategoryFilter — auto-store worker reads last_classification() after should_store()
- Phase 08.5-03: category tags stored as "category:{name}" prefix — searchable via existing tag filter
- Phase 08.5-03: store-low categories get stability=1.5 (vs 2.5 default for auto-store) — weaker salience signal for ephemeral-ish content
- Phase 08.5-04: apply_field_projection rewritten with dot-notation — one-level only, deeper paths silently ignored, non-object parents silently skipped
- Phase 08.5-04: composite_score = 0.5 * normalized_rrf + 0.5 * normalized_salience — single result gets 1.0

### Phase 08.4 Decisions
- Phase 08.4-01: parent_id TEXT REFERENCES memories(id) ON DELETE CASCADE — FK handles hard-delete cascade; soft-delete cascade done explicitly in GC worker
- Phase 08.4-01: ChunkingConfig defaults: enabled=true, max_chunk_chars=1024, overlap_sentences=2, min_content_chars=2048 — per locked CONTEXT.md
- Phase 08.4-01: delete_chunks_by_parent + get_chunks_by_parent store methods for explicit chunk lifecycle management
- Phase 08.4-02: unicode-segmentation (UAX#29) for sentence boundary detection — handles abbreviations, decimals, URLs correctly
- Phase 08.4-02: chunk_content returns empty Vec when disabled, below threshold, or single-chunk (no splitting needed)
- Phase 08.4-02: Context header format: [From: "topic", part X/Y]\n — makes chunks self-sufficient for retrieval
- Phase 08.4-03: Chunking wired into auto-store only (not explicit store) — per locked CONTEXT.md decision
- Phase 08.4-03: dedup_parent_chunks placed after salience threshold filter, before cursor pagination — correct ordering
- Phase 08.4-03: Dedup worker fetches 5 candidates (not 1) to allow sibling skipping while still finding true duplicates
- Phase 08.4-03: soft_delete_chunks_by_parents in GC worker — cascade after parent soft-delete (parent_ids from both salience prune + TTL expired)
- Phase 08.4-03: Chunk salience seeded at stability=2.5 (same as parent auto-store) — inherits parent's weaker-than-explicit signal

### Phase 08.3 Decisions
- Phase 08.3: Cargo workspace with memcp-core (library, [lib] name = "memcp") + memcp-bin (binary) — preserves all import paths without touching test files
- Phase 08.3: Backward-compat re-exports in lib.rs (pub use storage::store → makes crate::store available) — zero internal path changes needed
- Phase 08.3: CARGO_BIN_EXE_memcp env var replaced with runtime path lookup (env!("CARGO_MANIFEST_DIR") + ../../target/debug/memcp) in integration_test.rs and mcp_contract.rs
- Phase 08.3: include_str! paths updated for files that moved deeper (daemon.rs: ../contrib → ../../contrib)
- Phase 08.3: contrib/ and scripts/ copied into crates/memcp-core/ (include_str! resolves relative to source file, not workspace root)
- Phase 08.3: Domain directories are organizational only (not separate crates) — all in same memcp-core crate, no circular dependency issues
- Phase 08.3: benchmark/ stays as top-level module (not in any domain group) — it's tooling, not part of the main application

### Phase 08.2-03 Decisions
- Phase 08.2-03: Health server spawned after ready=true so first probe returns 200 immediately (avoids false 503s during model load)
- Phase 08.2-03: pipeline dropped in shutdown block to signal embedding drain; workers hold cloned senders so they complete in-flight work
- Phase 08.2-03: health_handle aborted after graceful shutdown timeout block so health stays live during drain period
- Phase 08.2-03: ready=false set before 10s timeout begins so orchestrators stop routing traffic immediately on SIGTERM
- Phase 08.2-03: process::exit(1) after 30s DB deadline — container restart policy is the correct recovery mechanism

### Phase 08.2-02 Decisions
- Phase 08.2-02: ResourceCapsConfig::default() used in MemoryService::new() — no breaking change to constructor signature; defaults are max_memories=None (unlimited)
- Phase 08.2-02: Cap enforcement is fail-open — count_live_memories() error logs warning and proceeds (Pitfall 4 from RESEARCH.md)
- Phase 08.2-02: search_memory limit clamps with min(user_limit as i64, max_search_results) as u32 — handles i64/u32 type boundary
- Phase 08.2-02: CLI store exits via std::process::exit(1) on cap exceeded — matches CLI convention for hard operational limits
- Phase 08.2-02: cmd_store gains config: &Config param following cmd_search(&store, &config, ...) pattern

### Phase 08.2-01 Decisions
- Phase 08.2-01: /health uses AtomicBool (Acquire ordering) — sub-ms, suitable for tight orchestrator probe loops; /status queries DB for dashboards
- Phase 08.2-01: Health server bind failure is non-fatal (warning log) — daemon starts MCP serve even if port 9090 is taken
- Phase 08.2-01: Both /health and /status are public (no auth) — per locked CONTEXT.md decision
- Phase 08.2-01: Separate port 9090 from MCP stdio transport — per locked CONTEXT.md decision
- Phase 08.2-01: axum::serve used (axum 0.8 API) rather than deprecated Server::bind

### Phase 08-05 Decisions
- Phase 08-05: 75% line coverage threshold (conservative start per RESEARCH.md) — bump after first CI measurement confirms actual baseline
- Phase 08-05: Golden tests gated on PR events only — saves CI time on pushes, catches regressions before merge
- Phase 08-05: cargo-llvm-cov via taiki-e/install-action — idiomatic CI install pattern
- Phase 08-05: Local dev uses port 5433 (Docker Postgres) consistent with project convention

### Phase 08-04 Decisions
- Phase 08-04: min_score thresholds for golden tests use RRF scale (0.01) not cosine similarity scale — RRF 1/(k+rank) produces ~0.016 for rank-1 with k=60; cosine 0.5 thresholds would always fail
- Phase 08-04: Golden test file at tests/search_quality.rs (top level) not tests/e2e/ — reinforces Phase 08-02 decision about Cargo subdirectory discovery
- Phase 08-04: OnceLock<Mutex<LocalEmbeddingProvider>> for shared fastembed model — std::sync::OnceLock (no extra dep), Mutex for interior mutability, shared across all 3 golden tests
- Phase 08-04: GoldenQuery extended with seed_content (what to store) and category (preference/fact/instruction/decision) — separates stored content from query topic; category enables targeted isolation tests

### Phase 08-03 Decisions
- Phase 08-03: RecallEngine no-extraction tier only recalls fact/summary type_hints — journey tests and other callers must use type_hint="fact" or "summary" for recall to work without extraction enabled
- Phase 08-03: McpTestClient must be redefined locally in each test crate — Rust integration test crates are isolated binaries and cannot share code across test files
- Phase 08-03: recall_memory MCP contract test asserts isError=false without requiring non-empty results — serve mode has no daemon for vector embeddings, BM25-only recall may return empty (correct behavior)
- Phase 08-03: Auto-store E2E tests target LogParser directly (not AutoStoreWorker.spawn()) — parser tests don't need DB/daemon, faster and more deterministic

### Phase 08-02 Decisions
- Phase 08-02: Integration test files go at tests/*.rs top level — Cargo does not auto-discover tests/ subdirectories without explicit #[path] wiring in a driver file
- Phase 08-02: Mock embedding inserts require all 7 required columns (id, memory_id, embedding, model_name, model_version, dimension, is_current, created_at, updated_at) — no ON CONFLICT unique constraint on (memory_id, model_name)
- Phase 08-02: GC tests need a live memory alongside soft-deleted ones to avoid min_memory_floor early-skip (count_live_memories() <= floor returns GcResult::skipped)
- Phase 08-02: recall_candidates FLOAT8 bug fixed — pgvector cosine distance returns FLOAT8 (f64) but code called row.get::<f32>(), fixed to try_get::<f64>().map(|v| v as f32)
- Phase 08-02: Summarization tests use #[test] (not #[sqlx::test]) — config and factory construction tests need no DB

### Phase 08-01 Decisions
- Phase 08-01: tests/unit.rs uses #[path] directives — Rust integration test module resolution for `mod foo;` looks at `tests/foo.rs`, not `tests/unit/foo.rs`; #[path] is the correct mechanism for subdirectory organization
- Phase 08-01: Six private helpers made pub for external test access: extract_agent_from_path, read_new_lines, is_jsonl, content_hash, cosine_similarity, format_relative_time — all pure-logic, safe to expose
- Phase 08-01: tests/common/helpers.rs uses `use memcp::store::MemoryStore as _;` — imports trait for method dispatch without polluting namespace

### Phase 07.1-01 Decisions
- Phase 07.1-01: Directory detection uses path.is_dir() at runtime — simpler than extension-absence heuristic from original plan
- Phase 07.1-01: scan_directory_jsonl is recursive — handles nested ~/.openclaw/agents/vita/sessions/ hierarchy
- Phase 07.1-01: --source is Vec<String> with prefix OR matching (not exact single-string per plan spec) — 'openclaw' matches 'openclaw/vita'
- Phase 07.1-01: Auto-discover ~/.claude/projects when watch_paths empty — zero-config for Claude Code users
- Phase 07.1-01: filter_mode default 'none' — auto-store works without Ollama; LLM filtering is opt-in via config

### Phase 07.11-02 Decisions
- Phase 07.11-02: recall_candidates uses string-format embedding vector ('[0.1,0.2,...]') with ::vector cast — consistent with hybrid_search approach
- Phase 07.11-02: MemoryService gains recall_config + extraction_enabled fields with defaults; set_recall_config() for post-construction wiring from main.rs
- Phase 07.11-02: recall_memory added to sandbox-safe tool list — context injection is safe for code execution sandboxes, not destructive
- Phase 07.11-02: Session cleanup added after idempotency key sweep in GC loop — single loop for all TTL-based cleanup

### Phase 07.11-01 Decisions
- Phase 07.11-01: RecallConfig follows IdempotencyConfig pattern exactly — serde(default) on all 5 fields, standalone Default impl, #[serde(default)] on Config field
- Phase 07.11-01: ensure_session uses INSERT ... ON CONFLICT DO UPDATE (not SELECT-then-INSERT) to avoid race conditions under concurrent recall
- Phase 07.11-01: clear_session_recalls resets last_active_at after DELETE to prevent immediate re-expiry by GC worker (Pitfall 3 from RESEARCH.md)
- Phase 07.11-01: recall_bump_salience does NOT update last_reinforced_at/reinforcement_count — passive implicit signal like touch_salience, not explicit reinforcement
- Phase 07.11-01: cleanup_expired_sessions binds idle_secs as string for ($1 || ' seconds')::INTERVAL cast — avoids SQL injection while supporting dynamic durations

### Phase 07.2-04 Decisions
- Phase 07.2-04: McpClient::spawn() delegates to spawn_with_env(vec![]) — backward compat preserved; McpTestClient passes DATABASE_URL override
- Phase 07.2-04: McpTestClient::cleanup() is explicit async method (not Drop) — simpler than block_in_place in Drop for this use case
- Phase 07.2-04: DROP DATABASE ... WITH (FORCE) — Postgres 13+ terminates active connections before drop
- Phase 07.2-04: Contract tests are #[tokio::test] not #[sqlx::test] — McpTestClient owns its own DB lifecycle
- Phase 07.2-04: Existing McpClient-based tests left on ambient DATABASE_URL — migration to McpTestClient deferred (not required for correctness)

### Phase 07.2-03 Decisions
- Phase 07.2-03: GC candidate threshold 1.5 in stress test (above default stability 1.0) — forces non-empty result exercising full LEFT JOIN query plan at 100k scale
- Phase 07.2-03: stress_test.rs bulk insert uses FNV-1a pre-computed in Rust (not pgcrypto sha256) — pgcrypto not available in ephemeral sqlx test databases; FNV-1a matches production algorithm
- Phase 07.2-03: 100-row batch size (500 params/query) for bulk inserts — safe within pg wire limit, avoids lock contention, fast enough for 90k inserts

### Phase 07.2-02 Decisions
- Phase 07.2-02: MemoryBuilder defaults are realistic (dark mode preference fact, test-agent source) not Lorem ipsum placeholders
- Phase 07.2-02: tags() accepts Vec<&str> for ergonomic call sites: tags(vec!["editor", "ui"]) without .to_string() clutter
- Phase 07.2-02: #[allow(dead_code)] on MemoryBuilder struct and impl — suppressed at source, not with global lint override

### Phase 07.2-01 Decisions
- Phase 07.2-01: from_pool() skips migrations — #[sqlx::test] runs sqlx::migrate!() automatically via migrator argument
- Phase 07.2-01: MIGRATOR exposed as pub static at crate root (src/lib.rs) so test crates can reference memcp::MIGRATOR
- Phase 07.2-01: store-level tests moved to tests/store_test.rs; MCP-over-stdio protocol tests stay in integration_test.rs unchanged
- Phase 07.2-01: test_persistence_across_restart removed — meaningless with ephemeral DBs, Postgres durability is not our code's responsibility
- Phase 07.2-01: clean_all() removed entirely — #[sqlx::test] isolation replaces all manual cleanup

### Roadmap Evolution
- Phases 01-06.3: Completed as originally planned
- Phase 06.4: Multi-actor provenance + topic exclusion — DONE
- Phase 06.5: CLI interface + daemon mode — DONE
- Phase 06.6: Auto-summarization — DONE
- GSD infrastructure bootstrapped from existing .planning/phases/ structure
- Phase 08.3 inserted after Phase 08: Modularize (URGENT)
- Phase 07.2 (Test Database) added to roadmap — separate test DB from main dev/production DB, inserted before Phase 08 Testing
- Phase 07.2: Adopted #[sqlx::test] for ephemeral per-test databases; store-level tests moved from MCP-over-stdio to direct store calls
- Phase 07.3 (Sidecar Status Indicator) added — daemon health visibility via CLI/skill/status line, inserted before Phase 08

### Key Decisions
- Phase 07.7-01: FNV-1a chosen for content_hash — deterministic, no new dependencies, cross-process stable
- Phase 07.7-01: Idempotency key registered AFTER insert with ON CONFLICT DO NOTHING — safe for concurrent requests
- Phase 07.7-01: idempotency_config on MemoryService uses default; set_idempotency_config() for runtime override
- Phase 07.7-01: cleanup_expired_idempotency_keys() called on every GC cycle (hourly) — no separate sweep needed
- Phase 07.7-01: Silent dedup return — same response shape as new store, caller cannot distinguish dedup hit (per CONTEXT.md)
- Phase 07.7-01: content_hash dedup check uses created_at window (NOW() - N seconds) so old created_at timestamps correctly fall outside the window
- Phase 07.6-02: tool_router_with_meta() post-processes macro-generated ToolRouter to inject _meta.allowed_callers into search_memory and store_memory; delete_memory and bulk_delete_memories intentionally excluded (destructive)
- Phase 07.6-02: #[rmcp::tool_handler(router = Self::tool_router_with_meta())] — macro router= accepts any Expr, so substituting the wrapper gives _meta in list_tools output without macro fight
- Phase 07.6-02: Inline JSON examples in tool descriptions (not JSON Schema) per CEX-04 — compact for sandbox context windows per CONTEXT.md locked decision
- Phase 07.6-01: SearchConfig stored as search_config field on MemoryService (mirrors salience_config pattern); constructor gains one param; main.rs passes config.search.clone()
- Phase 07.6-01: apply_field_projection duplicated in cli.rs (not extracted to shared util) — function is trivial and plan explicitly allowed duplication
- Phase 07.6-01: Salience filter placed at step 12.5 (after re-ranking, before cursor/take) — correct ordering per RESEARCH.md pitfall guidance; filtering before re-ranking would degrade quality
- Phase 07.6-01: effective_min = params.min_salience.or(config.search.default_min_salience).unwrap_or(0.0) — API param overrides config default, config default overrides 0.0 (no filter)
- Phase 07.6-01: Compact mode with --fields outputs JSON-per-line of projected object rather than fixed id/score/snippet format — more useful for programmatic consumers
- Phase 07.5-04: IPC dispatch uses type field: absent or "embed" = legacy embed (backward compat); "type":"rerank" = LLM reranking path
- Phase 07.5-04: rerank_via_daemon timeout is 5000ms vs 500ms for embed — LLM calls are much slower than local model inference
- Phase 07.5-04: noop response when daemon has no QI provider — CLI receives None and silently skips re-ranking (fail-open, no extra warning)
- Phase 07.5-04: create_qi_reranking_provider moved to daemon.rs as pub fn — follows create_embedding_provider / create_extraction_provider pattern; main.rs delegates
- Phase 07.5-04: re-ranking applied to ALL CLI searches (top 10 candidates) when daemon+QI available — not gated on explicit flag per gap closure decision
- Phase 07.5-03: hybrid_search_paged uses rrf_score keyset (not salience) — store layer has no salience data; CLI/MCP use salience_score for their cursors; both consistent within their layer
- Phase 07.5-03: application-level cursor filtering after salience re-ranking — keyset comparison must happen post-ranking since scores are computed outside the store
- Phase 07.5-03: candidate pool multiplier limit*3 first page, limit*5 with cursor — ensures enough results after cursor skip
- Phase 07.5-03: encode_search_cursor (offset-based) retained DEPRECATED; encode_search_keyset_cursor is canonical path going forward
- Phase 07.5-02: apply_feedback preserves reinforcement_count — feedback is a salience signal, not a FSRS reinforcement event
- Phase 07.5-02: flat multipliers (not retrievability-adjusted) for feedback — simpler and correct for explicit user feedback use case
- Phase 07.5-02: MCP feedback_memory tool uses store_error_to_result — MemcpError::Validation (invalid signal) surfaces cleanly
- Phase 07.5-01: vector_k=Some(60.0) matches MCP serve VECTOR_BASE_K=60.0 exactly — plan said "limit*3 same oversampling" but MCP serve actually uses fixed 60.0
- Phase 07.5-01: type_hint filter applied post-search in application layer — hybrid_search lacks type_hint WHERE clause; simpler and correct
- Phase 07.5-01: stale socket cleanup via connect-first pattern — if connect succeeds another daemon owns socket (skip); if fails, stale (remove+rebind)
- Phase 07.5-01: embed IPC listener spawned with provider_for_filter.clone() at step 3.7 in run_daemon — original Arc stays available for content filter
- Phase 07.5-00: wave0_07_5 feature flag gates test stubs calling non-existent methods (apply_feedback, hybrid_search_paged) — #[ignore] doesn't prevent compile errors
- Phase 07.5-00: test_cli_search_daemon_offline defers --json flag assertion to Plan 01; asserts current behavior (exit 0 + valid JSON stdout)
- Phase 07.5-00: docker-compose port corrected from 5432:5432 to 5433:5432 per MEMORY.md spec — local postgres occupies 5432
- Phase 07.4-03: DedupConfig uses enabled=true default — dedup on by default, same as GC; opt out via [dedup] enabled=false
- Phase 07.4-03: try_send used for dedup channel (non-blocking) so a backed-up dedup queue never stalls the embedding pipeline
- Phase 07.4-03: merge_duplicate transaction: UPDATE existing WHERE deleted_at IS NULL guards against re-merging already-deleted memories
- Phase 07.4-03: Serve path passes None for dedup_sender — dedup is daemon-only, keeping serve lightweight
- Phase 07.4-03: similarity.rs find_similar_memories() now filters deleted_at IS NULL (was missing — could return soft-deleted matches)
- Phase 07.4-02: CategoryFilter is opt-in via filter_mode = 'category' in memcp.toml — default (none) unchanged, conservative by design
- Phase 07.4-02: Fail-open on invalid custom regex: bad patterns skipped with tracing::warn, never panic
- Phase 07.4-02: LLM fallback field present in CategoryFilter constructor but deferred — heuristic-only per CONTEXT.md
- Phase 07.4-02: filtered_count() uses Arc<AtomicU64> with Ordering::Relaxed — stat tracking, not critical path
- Phase 07.4-01: Soft-delete via deleted_at TIMESTAMPTZ; hard purge after configurable grace period (default 30 days)
- Phase 07.4-01: GC never prunes below min_memory_floor=100 to protect small knowledge bases; prune budget = (live_count - floor)
- Phase 07.4-01: TTL-expired memories do not count against prune budget — they are removed regardless of budget
- Phase 07.4-01: hard_purge cascades to memory_embeddings, memory_salience, memory_consolidations in explicit order
- Phase 07.4-01: cmd_gc applies CLI flag overrides (--salience-threshold, --min-age-days) on top of config.gc defaults
- Phase 07.3-01: build_status() extracted from cmd_status for testability — integration tests assert JSON shape without capturing stdout
- Phase 07.3-01: Ingest tracking uses atomic SQL with date rollover (CASE WHEN ingest_date = today THEN +1 ELSE 1)
- Phase 07.3-01: scan_directory_jsonl made pub for reuse from daemon startup metadata write
- Phase 07.3-02: Status line script embedded via include_str! at compile time — no runtime file dependency
- Phase 07.3-02: Statusline commands bypass DB connection (no store needed for file operations)
- Phase 07-04: current_embedding_dimension() queries LIMIT 1 from memory_embeddings WHERE is_current=true — all current rows have same dim, no GROUP BY needed
- Phase 07-04: Cross-dimension switch requires --yes flag (destructive); same-dim switch does not (reversible via backfill)
- Phase 07-04: HNSW index dropped before purge in cross-dim case so daemon recreates with correct dimension on restart
- Phase 07-04: Unknown model name exits with full supported model list rather than silently failing
- Phase 07-03: Migration 010 drops vector(384) typed column and HNSW index; daemon recreates index at startup with configured dimension — brief ANN gap acceptable, pgvector falls back to exact search
- Phase 07-03: ensure_hnsw_index uses (embedding::vector(N)) functional index cast so pgvector can apply typed cosine ops on untyped column; idempotent (checks pg_indexes first)
- Phase 07-03: embedding_dimension stored on PostgresMemoryStore as Option<usize> — None for CLI path, set by daemon for future explicit cast support
- Phase 07-01: fastembed made optional via dep:fastembed syntax; default = ["local-embed"] preserves existing behavior
- Phase 07-01: Benchmark binary fully gated under #[cfg(feature = "local-embed")] — exits with clear error if feature missing
- Phase 07-02: model_dimension() as single source of truth for model→dimension mapping in embedding/mod.rs
- Phase 07-02: EmbeddingConfig gains local_model/openai_model/dimension fields with backward-compatible defaults
- Phase 07-02: OpenAIEmbeddingProvider::new() accepts Option<String> model + Option<usize> dimension; unknown model without dim override is a construction-time error
- Phase 07-02: SearchFilter::default() uses vec![0.0f32; 1] placeholder — callers always set query_embedding before use
- Provenance fields (actor, actor_type, audience) added now as schema groundwork; real identity wired in phase 12 (Auth)
- Topic exclusion: two-tier (regex patterns + semantic topics), ingestion-time filtering, default silent drop
- Exclusion hierarchy designed for server → tenant → user → agent, but only server-wide implemented initially
- hybrid_search uses audience post-filtering on fused results (not per-search-leg filtering) — simpler, sufficient until Phase 12 Auth
- CLI replaces MCP as primary agent interface — CLI commands are short-lived, daemon hosts background workers
- fastembed model (All-MiniLM-L6-v2) stays in daemon process (too heavy to reload per CLI invocation)
- CLI stores with embedding_status='pending', daemon processes async — clean separation
- `memcp serve` (MCP mode) stays for backwards compatibility — CLI+daemon is the primary path going forward
- Agent instruction pattern: CLAUDE.md tells agent to run `memcp --help` once at session start, then use CLI for all memory ops
- fastembed All-MiniLM-L6-v2: 87MB on disk (~similar in RAM), cached at `~/Library/Caches/memcp/models/`
- Switching embedding models mid-use has 3 issues: (1) dimension mismatch — vector(384) column hardcoded in migration 002, (2) semantic incompatibility — can't cosine-compare embeddings from different models, (3) full backfill required. `embed switch-model` handles same-dimension swaps; cross-dimension switching deferred to Phase 07 (Modularity)
- Phase 06.6 (Auto-Summarization): auto-store summarizes AI responses via Ollama. Agents can still store raw unsummarized content directly. Provenance: `type_hint: "summary"` + tag `"summarized"` for summaries
- Summarization is daemon-mode only (MCP serve passes None) — keeps serve mode lightweight
- Fail-open: if summarization fails, raw content is stored with warning log (no data loss)

### Phase 06.6 Deliverables
- SummarizationConfig in config.rs (enabled, provider, ollama/openai settings, max_input_chars, prompt_template)
- SummarizationProvider trait with summarize() + model_name() methods
- OllamaSummarizationProvider — calls /api/chat with system prompt + user content
- OpenAISummarizationProvider — calls /chat/completions, supports any OpenAI-compatible API
- create_summarization_provider factory (None when disabled, Err when misconfigured)
- Auto-store worker: summarizes assistant responses, stores user messages raw
- Provenance: type_hint="summary" + tag "summarized" for summarized entries
- Daemon creates and wires summarization provider; serve mode passes None
- 4 unit tests for summarization module, 66 total tests passing

### Phase 06.5 Deliverables
- CLI subcommands: store, search, list, get, delete, reinforce, status (all JSON output to stdout)
- Daemon mode: embedding pipeline, extraction pipeline, consolidation worker, auto-store sidecar
- Daemon heartbeat every 30s to daemon_status table; CLI warns on stderr if daemon not running
- Pending work polling every 10s in daemon (catches CLI-stored memories)
- `memcp daemon install` for macOS (launchd) and Linux (systemd) service installation
- MCP tool descriptions trimmed to one-liners; server instructions field added
- Workspace CLAUDE.md updated with CLI-first workflow instructions
- Provider creation functions (embedding, extraction) DRY'd into daemon module
