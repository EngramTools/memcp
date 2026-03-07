# Roadmap — Milestone 1: Core Memory Server

## Phase 01: Foundation
- **Goal**: Project scaffolding, basic Rust/Tokio/rmcp setup
- **Status**: DONE
- **Depends on**: —

## Phase 02: Core Memory
- **Goal**: CRUD operations for memories (store, get, update, delete, list)
- **Status**: DONE
- **Depends on**: Phase 01

## Phase 03: Persistence
- **Goal**: PostgreSQL storage backend with migrations
- **Status**: DONE
- **Depends on**: Phase 02

## Phase 04: Embeddings
- **Goal**: Embedding generation pipeline (fastembed local + OpenAI provider)
- **Status**: DONE
- **Depends on**: Phase 03

## Phase 05: Vector Search
- **Goal**: Semantic similarity search via pgvector
- **Status**: DONE
- **Depends on**: Phase 04

## Phase 06: Hybrid Search + Salience
- **Goal**: BM25 + vector hybrid search with salience scoring (recency, access, semantic, reinforcement)
- **Status**: DONE
- **Depends on**: Phase 05

## Phase 06.1: Search Enrichment
- **Goal**: Entity/fact extraction pipeline, consolidated search results
- **Status**: DONE (PR #1)
- **Depends on**: Phase 06

## Phase 06.2: Query Intelligence
- **Goal**: Query expansion + LLM re-ranking for search
- **Status**: DONE (PR #2)
- **Depends on**: Phase 06.1

## Phase 06.3: Memory Benchmarking
- **Goal**: Benchmark harness, CLI, and CI integration for search quality
- **Status**: DONE (PR #3)
- **Depends on**: Phase 06.2

## Phase 06.4: Provenance + Topic Exclusion
- **Goal**: Multi-actor provenance tracking (actor, actor_type, audience columns) and ingestion-time content filtering (regex patterns + semantic topic exclusion)
- **Status**: DONE
- **Depends on**: Phase 06.3
- **Requirements:** [PROV-01, PROV-02, EXCL-01, EXCL-02, EXCL-03, EXCL-04, EXCL-05, EXCL-06]
- **Plans:** 3 plans

Plans:
- [x] 06.4-01-PLAN.md — Schema migration + provenance plumbing through structs, SQL, and MCP tools
- [x] 06.4-02-PLAN.md — Content filter module (regex + semantic topic exclusion)
- [x] 06.4-03-PLAN.md — Integration: wire content filter into all ingestion paths

## Phase 06.5: CLI Interface + Daemon Mode
- **Goal**: CLI subcommands for all memory operations, daemon mode for background workers, trim MCP descriptions, agent instructions for CLI usage. Eliminates MCP schema overhead (~10x token reduction per memory operation).
- **Status**: DONE
- **Depends on**: Phase 06.4
- **Requirements:** [CLI-01, CLI-02, CLI-03, CLI-04, CLI-05, CLI-06, CLI-07, CLI-08, CLI-09]
- **Plans:** 3 plans

Requirements:
- CLI-01: `memcp store` — write memory to DB with `embedding_status='pending'`, exit immediately
- CLI-02: `memcp search` — hybrid search with salience scoring, JSON output
- CLI-03: `memcp list` — list/filter memories with pagination
- CLI-04: `memcp get` / `memcp delete` / `memcp reinforce` — single-memory operations
- CLI-05: `memcp daemon` — long-running process hosting embedding pipeline, extraction pipeline, consolidation worker, and auto-store sidecar
- CLI-06: `memcp --help` — concise usage guide suitable for single-read agent consumption
- CLI-07: Trim MCP tool descriptions (shorter param docs, move guidance to instructions field)
- CLI-08: CLAUDE.md / agent config instructions for CLI-first usage
- CLI-09: Remove memcp from MCP server config after CLI is validated

Plans:
- [x] 06.5-01-PLAN.md — CLI framework + all subcommands (store, search, list, get, delete, reinforce, status) [Wave 1]
- [x] 06.5-02-PLAN.md — Daemon mode + service management (embedding/extraction/consolidation/auto-store workers, heartbeat, install) [Wave 1]
- [x] 06.5-03-PLAN.md — MCP trim + agent instructions (shorter descriptions, CLAUDE.md CLI workflow) [Wave 2, depends on 01+02]

## Phase 06.6: Auto-Summarization
- **Goal**: Auto-store sidecar summarizes AI responses via local LLM (Ollama) before storing. Agents can also store raw unsummarized content directly. Provenance distinguishes summarized vs raw memories.
- **Status**: DONE
- **Depends on**: Phase 06.5
- **Requirements:** [SUM-01, SUM-02, SUM-03, SUM-04, SUM-05]
- **Plans:** 2 plans

Requirements:
- SUM-01: Auto-store sidecar summarizes AI assistant responses before storing (configurable: on/off, provider, model)
- SUM-02: Raw unsummarized storage still available — agent can store directly via CLI/MCP without summarization
- SUM-03: Provenance distinguishes summarized vs raw: `type_hint: "summary"` + tag `"summarized"` for summaries, original `type_hint` preserved for raw
- SUM-04: Summarization uses existing Ollama/OpenAI provider infrastructure (zero API cost with local Ollama)
- SUM-05: Configurable summarization prompt template (what to extract/compress from AI responses)

Plans:
- [x] 06.6-01-PLAN.md — Summarization module: trait, Ollama/OpenAI providers, config [Wave 1]
- [x] 06.6-02-PLAN.md — Auto-store integration: wire summarization into worker, provenance tagging, daemon wiring [Wave 2, depends on 01]

## Phase 07: Modularity
- **Goal**: Cargo features, compile-time toggles for optional dependencies. Ship with/without bundled fastembed model (`--features local-embed`). Dynamic vector column dimensions (fix hardcoded 384-dim). Safe model switching with automatic dimension migration.
- **Status**: DONE (4/4 plans done)
- **Depends on**: Phase 06.6
- **Requirements:** [MOD-01, MOD-02, MOD-03, MOD-04, MOD-05, MOD-06]
- **Plans:** 5 plans (4 complete + 1 gap closure)

Requirements:
- MOD-01: `fastembed` is optional behind `local-embed` Cargo feature (default on). `--no-default-features` builds without it.
- MOD-02: Embedding model and dimension configurable via `memcp.toml` (`embedding.local_model`, `embedding.openai_model`, `embedding.dimension`)
- MOD-03: Model dimension registry maps known model names to their output dimensions
- MOD-04: Database schema supports any vector dimension (untyped `vector` column, migration removes `vector(384)` constraint)
- MOD-05: Daemon creates HNSW index at startup matching configured dimension
- MOD-06: `memcp embed switch-model` handles dimension changes: detects mismatch, purges incompatible embeddings, drops/recreates HNSW index

Plans:
- [x] 07-01-PLAN.md — Cargo features + conditional compilation (fastembed behind `local-embed` feature) [Wave 1] — commits 6756fe3..e07e47f
- [x] 07-02-PLAN.md — Configurable embedding models + dimension registry (config fields, provider changes) [Wave 1] — commit e07e47f
- [x] 07-03-PLAN.md — Dynamic schema + HNSW index management (migration, startup index creation) [Wave 2, depends on 02] — commits 7007e48..ffcad16
- [x] 07-04-PLAN.md — Safe model switching with dimension migration (enhanced switch-model CLI, purge flow) [Wave 2, depends on 02+03] — commits c489c6d..c00ce5d

## Phase 07.1: Auto-Store Claude Code Integration
- **Goal**: Directory/glob watching for auto-store sidecar (picks up new Claude Code session JSONL files automatically), change default filter_mode to "none" (works without Ollama), add `--source` flag to search CLI.
- **Status**: DONE (1/1 plan done)
- **Depends on**: Phase 07
- **Requirements:** [AS-01, AS-02, AS-03]

Requirements:
- AS-01: Auto-store sidecar watches a directory and picks up new `.jsonl` files as they appear (glob/directory watching support)
- AS-02: Default `filter_mode` changed from `"llm"` to `"none"` — works out of the box without Ollama; users opt in to LLM filtering via config
- AS-03: `memcp search` CLI gains `--source` flag for filtering by memory source (already supported in store layer, just missing from CLI)

Plans:
- [x] 07.1-01-PLAN.md — Directory watching + filter default + --source flag — commit b6fc620

## Phase 07.2: Test Database
- **Goal**: Separate test database from the main development/production database so tests and dev usage don't interfere with each other
- **Status**: DONE (4/4 plans done)
- **Depends on**: Phase 07.1
- **Plans:** 5 plans (4 complete + 1 gap closure)

Plans:
- [x] 07.2-01-PLAN.md — Test DB isolation via `#[sqlx::test]`: from_pool constructor, per-test ephemeral databases, parallel-safe tests
- [x] 07.2-02-PLAN.md — Builder helpers in tests/common/ + store_test.rs refactor [Wave 1, depends on 01]
- [x] 07.2-03-PLAN.md — 100k memory stress test with #[ignore] [Wave 1, depends on 02]
- [x] 07.2-04-PLAN.md — McpTestClient temp DB lifecycle + MCP protocol contract tests [Wave 2, depends on 01+02]

## Phase 07.3: Sidecar Status Indicator
- **Goal**: Visible indicator that the memcp daemon/sidecar is running. Enhanced `memcp status` with `--pretty` one-liner and `--check` deep health check. Claude Code status line integration. Shows daemon health, sidecar activity, last ingestion timestamp, pending counts, embedding model info.
- **Status**: DONE (2/2 plans done)
- **Depends on**: Phase 07.2
- **Requirements:** [SSI-01, SSI-02, SSI-03, SSI-04, SSI-05, SSI-06]

Requirements:
- SSI-01: `memcp status` JSON output enriched with sidecar metrics (last_ingest_at, ingest_count_today, watched_file_count) and model info (name, dimension)
- SSI-02: `memcp status --pretty` — human-readable one-liner showing daemon state, uptime, pending counts, last ingest time, total memories
- SSI-03: `memcp status --check` — deep health check probing DB, Ollama, model cache, and watch paths
- SSI-04: Auto-store worker writes `last_ingest_at` to daemon_status after each successful memory store
- SSI-05: Claude Code status line script showing daemon health indicator (configurable: ingest time, pending count, or state only)
- SSI-06: `memcp statusline install` copies script to `~/.claude/scripts/` and prints settings.json instructions

Plans:
- [x] 07.3-01-PLAN.md — Schema + daemon tracking + enhanced CLI status (migration, auto-store ingest tracking, --pretty, --check) [Wave 1] — commits c2023ad..0239ae7
- [x] 07.3-02-PLAN.md — Claude Code status line script + install command [Wave 2, depends on 01] — commits 2ef0934..c3dfd5a

## Phase 07.4: Memory Hygiene
- **Goal**: Reduce noise and prevent unbounded memory growth. Salience-threshold garbage collection with optional TTLs. Semantic deduplication on ingest via embedding similarity. Category-aware auto-store filtering (skip tool-call narration, keep decisions/preferences/architecture).
- **Status**: DONE (3/3 plans done)
- **Depends on**: Phase 07.3
- **Requirements:** [HYG-01, HYG-02, HYG-03, HYG-04, HYG-05, HYG-06]
- **Plans:** 3/3 plans complete

Requirements:
- HYG-01: Salience-threshold GC — background worker prunes memories below configurable salience threshold after configurable age (e.g., salience < 0.3 and older than 30 days)
- HYG-02: Optional TTL support — memories can have an `expires_at` timestamp; GC worker deletes expired memories
- HYG-03: Semantic deduplication on ingest — before storing, compute embedding similarity against recent memories; if similarity > threshold (e.g., 0.95), merge or skip
- HYG-04: Category-aware auto-store filtering — classify incoming content (decision, preference, architecture, tool-narration, ephemeral) and skip low-value categories
- HYG-05: GC dry-run mode — `memcp gc --dry-run` shows what would be pruned without deleting
- HYG-06: GC metrics in `memcp status` — show last GC run, items pruned, dedup merges

Plans:
- [x] 07.4-01-PLAN.md — GC module: migration, soft-delete, TTL, dry-run CLI, status metrics [Wave 1]
- [x] 07.4-02-PLAN.md — Category-aware auto-store filtering (CategoryFilter + tool narration patterns) [Wave 1]
- [x] 07.4-03-PLAN.md — Async semantic deduplication post-embedding + daemon GC schedule [Wave 2, depends on 01]

## Phase 07.5: Search Consistency & Feedback
- **Goal**: Unify search behavior across MCP serve and CLI via daemon IPC for embeddings. Add explicit feedback loop (useful/irrelevant signals) wired into FSRS salience scoring. Replace offset-based search pagination with keyset cursor-based for scalability.
- **Status**: DONE (5/5 plans done)
- **Depends on**: Phase 07.4
- **Requirements:** [SCF-01, SCF-02, SCF-03, SCF-04, SCF-05]
- **Plans:** 5/5 plans complete

Requirements:
- SCF-01: CLI search uses the same hybrid search pipeline as MCP serve — embeddings, salience re-ranking, query expansion all available from CLI (daemon provides embedding/LLM services)
- SCF-02: `memcp feedback <id> useful|irrelevant` — explicit relevance signal that adjusts FSRS stability/difficulty for the memory
- SCF-03: Search results include a feedback hint (e.g., memory ID) so agents can easily provide feedback after using a result
- SCF-04: Cursor-based pagination for search and list — keyset pagination using (salience_score, id) or (created_at, id) pairs instead of OFFSET
- SCF-05: Deprecate offset-based pagination with migration path (keep working for one version, warn, then remove)

Plans:
- [x] 07.5-00-PLAN.md — Wave 0 test stubs for all SCF requirements (Nyquist compliance) [Wave 0]
- [x] 07.5-01-PLAN.md — Daemon embed IPC + CLI search parity with full hybrid pipeline + output formats [Wave 1, depends on 00]
- [x] 07.5-02-PLAN.md — Feedback command (useful/irrelevant) + FSRS failed-review + MCP tool [Wave 1, depends on 00]
- [x] 07.5-03-PLAN.md — Keyset cursor-based search pagination + offset deprecation [Wave 2, depends on 00+01]
- [x] 07.5-04-PLAN.md — Gap closure: rerank IPC + CLI wiring for SCF-01 query expansion parity [Wave 3, depends on 01] — commits e8b2513..b91152d

## Phase 07.6: Code Execution Support
- **Goal**: Optimize memcp's MCP tools for Anthropic's programmatic tool calling pattern. Add server-side field projection and filtering so code execution sandboxes receive minimal structured data. Design internal API boundaries for future sandbox exposure. Currently supported by Claude Opus/Sonnet 4.5+ via `code_execution_20260120`.
- **Status**: DONE (2/2 plans done)
- **Depends on**: Phase 07.5
- **Requirements:** [CEX-01, CEX-02, CEX-03, CEX-04]
- **Reference**: https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling
- **Plans:** 2/2 plans complete

Requirements:
- CEX-01: `search_memory` gains `fields` param — field projection to return only requested fields (e.g., `["content", "tags"]` omits full metadata), reducing token payload
- CEX-02: `search_memory` gains `min_salience` param — server-side salience threshold filtering before results reach the model/sandbox
- CEX-03: `allowed_callers` annotation on MCP tool definitions — mark search_memory and store_memory as callable from code execution (`["code_execution_20260120"]` or `["direct", "code_execution_20260120"]`)
- CEX-04: Tool descriptions include structured output format documentation (JSON schema of return types) so models can reliably deserialize results in code execution sandboxes

Plans:
- [x] 07.6-01-PLAN.md — Field projection + salience threshold filtering (MCP + CLI + config) [Wave 1] — commits 1b0d8ef..74bb33e
- [x] 07.6-02-PLAN.md — Tool description output docs + allowed_callers metadata [Wave 2, depends on 01] — commits ead5d4d..80546cd

## Phase 07.7: Idempotent Tool Operations
- **Goal**: API-level idempotency for all memcp MCP tool operations. Fallback retry in code execution sandboxes and MCP transports can cause duplicate calls with identical arguments. Add idempotency keys, content-hash deduplication on store, and safe retry semantics so duplicate tool calls are no-ops.
- **Status**: DONE (1/1 plans done)
- **Depends on**: Phase 07.6
- **Requirements:** [IDP-01, IDP-02, IDP-03]
- **Plans:** 1/1 plans complete

Requirements:
- IDP-01: `store_memory` deduplicates on content hash — if identical content already exists within a configurable time window (default 60s), return existing memory ID instead of creating duplicate
- IDP-02: Optional `idempotency_key` param on `store_memory` — caller-provided key that guarantees at-most-once storage; repeated calls with same key return original result
- IDP-03: `search_memory` and `delete_memory` are naturally idempotent (read-only / delete-if-exists) — document this contract explicitly in tool descriptions

Plans:
- [x] 07.7-01-PLAN.md — TDD: content-hash dedup, idempotency keys, idempotent delete, tool description updates [Wave 1]

## Phase 07.11: Recall
- **Goal**: Automatic context injection from memory. New `recall` endpoint takes a query + session ID, returns threshold-filtered memories not yet seen in this session. Tiered strategy: with extraction enabled, search against extracted_facts for compact results; without extraction, filter to assistant/fact memories only (skip user-question memories). Session-memory join table prevents re-injection within a conversation; caller can signal compaction/clear to reset. Recalled memories get implicit salience reinforcement.
- **Status**: DONE (2/2 plans done)
- **Depends on**: Phase 07.6
- **Requirements:** [RCL-01, RCL-02, RCL-03, RCL-04, RCL-05, RCL-06]
- **Plans:** 2/2 plans complete

Requirements:
- RCL-01: `memcp recall` CLI + MCP tool — takes query text + session_id, embeds query, searches memories above relevance threshold, excludes already-recalled memories for this session, returns results
- RCL-02: `session_recalls` table (migration) — session_id TEXT, memory_id UUID FK, recalled_at TIMESTAMPTZ, relevance FLOAT; indexed on session_id for fast dedup lookups
- RCL-03: Configurable cap (default 3) and relevance threshold (default 0.7) via `recall.max_memories` and `recall.min_relevance` in memcp.toml
- RCL-04: Tiered recall strategy — extraction enabled: search `extracted_facts` column for compact fact injection; extraction disabled: filter to `type_hint IN ('fact', 'summary')` or `role=assistant` memories, skip user-question memories
- RCL-05: Compaction-aware dedup — `recall` accepts optional `reset=true` param to clear session recall history (caller signals context was compacted/cleared)
- RCL-06: Implicit salience reinforcement — each recall bumps FSRS stability for the recalled memory (bridges the feedback gap without requiring explicit signals)

Plans:
- [x] 07.11-01-PLAN.md — Migration 014 (sessions + session_recalls) + RecallConfig + store-layer session/salience methods [Wave 1]
- [x] 07.11-02-PLAN.md — RecallEngine module + CLI recall + MCP recall_memory tool + session auto-expiry in GC [Wave 2, depends on 01]

## Phase 08: Testing
- **Goal**: Integration tests, E2E against real Postgres, coverage. Codify all known-good behaviors before hardening. Rewrite all ~98 existing tests with consistent patterns. Move all inline #[cfg(test)] modules to top-level tests/ directory. Add coverage enforcement to CI.
- **Status**: DONE (5/5 plans done)
- **Depends on**: Phase 07.11
- **Requirements:** [TST-01, TST-02, TST-03, TST-04, TST-05, TST-06, TST-07, TST-08]
- **Plans:** 5/5 plans complete

Requirements:
- TST-01: All 12 inline `#[cfg(test)]` modules migrated from src/ to tests/unit/ — no test code remains in source files
- TST-02: Test infrastructure: tests/common/ with MemoryBuilder (expanded), golden dataset loader, shared helpers — reusable across all test categories
- TST-03: Integration tests for uncovered modules: recall, gc/dedup, feedback, summarization, embedding pipeline lifecycle
- TST-04: E2E journey tests: store → embed → search → recall → salience decay → reinforcement full flow against real Postgres
- TST-05: MCP contract tests using McpTestClient: all MCP tool operations tested via stdio protocol
- TST-06: Golden dataset for search quality regression: fixture file with known queries → expected results, tested with real fastembed
- TST-07: Auto-store sidecar E2E: simulated JSONL conversation input → verify memories ingested correctly
- TST-08: CI coverage enforcement: cargo-llvm-cov with threshold, GitHub Actions integration, parallel test execution (no --test-threads=1)

Plans:
- [x] 08-01-PLAN.md — Test infrastructure + unit test migration (common helpers, move all 12 inline modules to tests/unit/) [Wave 1]
- [x] 08-02-PLAN.md — Integration tests: recall, gc/dedup, feedback, summarization, embedding pipeline [Wave 1]
- [x] 08-03-PLAN.md — E2E journey + MCP contract tests + auto-store sidecar E2E [Wave 2, depends on 01]
- [x] 08-04-PLAN.md — Golden dataset search quality regression [Wave 2, depends on 01]
- [x] 08-05-PLAN.md — CI coverage enforcement + cleanup [Wave 3, depends on 01+02+03+04]

## Phase 08.1: Regression Suite & Manual QA
- **Goal**: ~~Automated regression tests for all core flows.~~ Largely delivered by Phase 08 (plans 02-05). QA playbook folded into Phase 09 (Documentation).
- **Status**: DROPPED (merged into Phase 08 + Phase 09)
- **Depends on**: Phase 08
- **Note**: Regression suite goals (store→search→recall cycle, dedup, salience decay, GC, feedback, field projection, content filtering) were covered by Phase 08 plans 02-05. Manual QA playbook moved to Phase 09 with AI QA agent support.

## Phase 08.2: Container Lifecycle
- **Goal**: Health check endpoint, graceful shutdown for container orchestrators (Fly/Railway), startup probes, resource caps (max memory entries per instance, max embedding batch size). Required before engram.host hosting.
- **Status**: DONE (3/3 plans done)
- **Depends on**: Phase 08
- **Requirements:** [CL-01, CL-02, CL-03, CL-04]
- **Plans:** 3/3 plans complete

Requirements:
- CL-01: `/health` HTTP endpoint (200/503 liveness) and `/status` endpoint (component breakdown: db, embeddings, hnsw + resource usage vs limits) on separate configurable port
- CL-02: SIGTERM triggers graceful shutdown: reject new requests, flush pending embeddings, close DB connections, exit cleanly within 10s timeout
- CL-03: Configurable resource caps (max_memories, max_embedding_batch_size, max_search_results, max_db_connections) enforced in both MCP serve and CLI paths
- CL-04: Startup readiness with DB connection retry (exponential backoff up to 30s) — `/health` returns 503 until DB connected + migrations applied

Plans:
- [x] 08.2-01-PLAN.md — Config structs + health HTTP server module (axum /health + /status) [Wave 1]
- [x] 08.2-02-PLAN.md — Resource cap enforcement in MemoryService + CLI [Wave 1, depends on 01]
- [x] 08.2-03-PLAN.md — Daemon wiring: health server spawn, graceful shutdown, DB startup retry [Wave 2, depends on 01+02]

## Phase 08.3: Modularize (INSERTED)
- **Goal**: Restructure memcp from single flat crate (~21k LOC) into Cargo workspace with memcp-core library crate and thin memcp binary crate. Organize subsystems into domain-layer directories (storage/, intelligence/, pipeline/, transport/) with clear boundaries and documentation. No new capabilities — structural reorganization only.
- **Status**: DONE (3/3 plans done)
- **Depends on**: Phase 08
- **Plans:** 3/3 plans complete

Plans:
- [x] 08.3-01-PLAN.md — Cargo workspace creation: workspace root, memcp-core library crate, memcp binary crate, move all files [Wave 1] — commit 2186ea4
- [x] 08.3-02-PLAN.md — Domain reorganization: move subsystems into storage/, intelligence/, pipeline/, transport/ with backward-compat re-exports [Wave 2, depends on 01] — commit 3ac573f
- [x] 08.3-03-PLAN.md — Module documentation + ARCHITECTURE.md + public API curation [Wave 2, depends on 01+02] — commit f0e9ac7

## Phase 08.4: Memory Chunking
- **Goal**: Split long content (conversations, documents) into overlapping chunks with separate vectors for better retrieval granularity. Configurable chunk size, overlap, and strategy. Standard RAG pattern that improves search quality on long-form content.
- **Status**: DONE (3/3 plans done)
- **Depends on**: Phase 08.3
- **Origin**: Deferred from Phase 04 (Embeddings)
- **Plans:** 3/3 plans complete

Plans:
- [x] 08.4-01-PLAN.md — Schema migration (parent_id, chunk_index, total_chunks) + ChunkingConfig + store methods [Wave 1] — commit 7a63d4b
- [x] 08.4-02-PLAN.md — Chunking module: sentence splitter (unicode-segmentation), context headers, chunk_content API [Wave 1] — commit 2962cfb
- [x] 08.4-03-PLAN.md — Integration: auto-store chunking, search dedup (prefer chunks over parents), dedup-worker sibling skip, GC cascade [Wave 2, depends on 01+02] — commit 48357c9

## Phase 08.5: API & Pipeline Polish
- **Goal**: Batch of small improvements across store, search, and pipeline. `--wait`/`--sync` flag for blocking store (waits for embedding completion). Auto-GC trigger when approaching resource caps (instead of hard-fail). Configurable re-embedding policy (skip re-embed on tag-only updates). LLM-based category expansion for auto-store filtering (beyond heuristic tool-narration patterns). Nested field projection with dot-notation paths (e.g., `metadata.source`). Scored relevance filtering (0-1 float instead of binary keep/drop).
- **Status**: Done
- **Depends on**: Phase 08.3
- **Origin**: Deferred from Phases 04, 06.5, 07.4, 07.6, 08.2
- [x] 08.5-01-PLAN.md — Sync store (--wait/oneshot) + re-embedding policy (reembed_on_tag_change) + ResourceLimitsConfig + auto-GC [Wave 1] — commit fa0a9d1
- [x] 08.5-02-PLAN.md — Capacity thresholds + auto-GC (implemented inline with Plan 01) [Wave 2] — commit fa0a9d1
- [x] 08.5-03-PLAN.md — LLM category classification (10-category taxonomy, per-category actions, category tags) [Wave 1] — commit fa0a9d1
- [x] 08.5-04-PLAN.md — Dot-notation field projection + composite_score (0-1 blended relevance) [Wave 2] — commit fa0a9d1

## Phase 08.6: AI Brain Curation
- **Goal**: Periodic self-maintenance daemon worker where the AI reviews its own memories — merges related entries, strengthens important ones, flags outdated ones, cleans up low-value content. Configurable curation schedule (daily/weekly). Differentiator feature for engram.host.
- **Status**: DONE
- **Depends on**: Phase 08.3
- **Origin**: Deferred from Phase 06.1 (Search Enrichment)
- **Plans:** 5/5 plans complete

Plans:
- [x] 08.6-01-PLAN.md — Curation foundation: migration, CurationConfig, CurationProvider trait, AlgorithmicCurator [Wave 1] — commit 18343bb
- [ ] 08.6-05-PLAN.md — Gap closure: true dry-run mode for --propose flag [Wave 1]

## Phase 08.7: Multi-Model Embeddings
- **Goal**: Run multiple embedding models simultaneously — fast local model for bulk ingestion, quality API model for important memories. Tiered embedding strategy with automatic model selection based on memory importance/type. Extends Phase 07 single-model modularity to concurrent multi-model.
- **Status**: DONE
- **Depends on**: Phase 08.3
- **Origin**: Deferred from Phase 04 (Embeddings)
- **Plans:** 5 plans (4 complete + 1 gap closure)

Plans:
- [x] 08.7-01-PLAN.md — Config structs (EmbeddingTierConfig, RoutingConfig, PromotionConfig) + migration 017 (tier column) + EmbeddingRouter + store methods [Wave 1]
- [x] 08.7-02-PLAN.md — Multi-tier pipeline wiring: router-based EmbeddingPipeline, daemon provider construction, auto-store routing [Wave 2, depends on 01]
- [x] 08.7-03-PLAN.md — Promotion sweep worker: periodic daemon task promoting important memories to quality tier [Wave 2, depends on 01]
- [x] 08.7-04-PLAN.md — Dual-query search: per-tier query embedding, multi-pass hybrid search, RRF merge, lazy quality optimization [Wave 3, depends on 01+02+03]

## Phase 08.8: Plugin Support Primitives
- **Goal**: Capabilities required by the OpenClaw memcp plugin. `memcp annotate` command to tag and boost salience on existing memories by ID. Auto-store emits memory IDs so callers can annotate what was just stored. Temporal event time extraction on ingest (parses references like "when I was 6", "in 2019" into structured metadata). Workspace scoping (`workspace` column) for multi-workspace isolation. Recall output improvements (--first preamble, truncation, related context hints).
- **Status**: DONE (5/5 plans done)
- **Depends on**: Phase 08.7
- **Origin**: memcp scope additions from OpenClaw plugin design (engram/.planning/memcp-openclaw-plugin-design.md)
- **Plans:** 5/5 plans complete

Plans:
- [x] 08.8-01-PLAN.md — Migration 018 (event_time, workspace columns) + Config structs + Memory/CreateMemory field additions [Wave 1]
- [x] 08.8-02-PLAN.md — Annotate command (CLI + MCP, tag append/replace, salience absolute/multiplier, diff output) [Wave 2, depends on 01]
- [x] 08.8-03-PLAN.md — Workspace scoping + temporal event-time regex extraction + CLI/MCP output field updates [Wave 2, depends on 01]
- [x] 08.8-04-PLAN.md — Auto-store ID emission (.ids.jsonl companion file) + temporal LLM background worker + daemon wiring [Wave 3, depends on 01+03]
- [x] 08.8-05-PLAN.md — Recall output improvements (--first preamble, truncation, related context hints) [Wave 3, depends on 01+02+03]

## Phase 08.9: Query-less Recall
- **Goal**: `memcp recall --first` without a query. Ranks memories by salience (primary) + recency (secondary) for cold start injection. Returns top N memories without requiring an embedding query. Includes project summary if tagged `project-summary`. Required by OpenClaw plugin's two-phase injection (cold start before first user message).
- **Status**: DONE (2/2 plans done)
- **Depends on**: Phase 08.8
- **Origin**: engram/.planning/memcp-knowledge-layer-vision.md (Two-Phase Context Injection, lines 96-134)
- **Plans:** 2/2 plans complete

Plans:
- [ ] 08.9-01-PLAN.md — Store methods (recall_candidates_queryless, fetch_project_summary) + RecallEngine.recall_queryless() + RecallResult.summary field [Wave 1]
- [ ] 08.9-02-PLAN.md — CLI (optional query, --limit flag, summary output) + MCP (optional query, first/limit params, tool description) [Wave 2, depends on 01]

## Phase 08.10: Memory Content Updates
- **Goal**: `memcp update <id> "new content"` CLI command for in-place memory content replacement. stdin support for multi-paragraph updates. Re-triggers embedding pipeline for updated content. Primary use case: evolving project summaries that agents maintain as living documents.
- **Status**: DONE (1/1 plans done)
- **Depends on**: Phase 08.8
- **Origin**: engram/.planning/memcp-knowledge-layer-vision.md (Memory Editing, lines 186-198)
- **Plans:** 1/1 plans complete

Plans:
- [ ] 08.10-01-PLAN.md — Commands::Update CLI + --stdin support on store/update + cmd_update handler [Wave 1]

## Phase 08.11: Warm Recall & Session-Aware Ranking
- **Goal**: Session-aware ranking enhancements for `memcp recall`. Tag-affinity boosting (`--boost-tags`) lets callers pass context tags (channel, agent, topic) that give a soft ranking bonus. Session topic accumulation tracks recalled memory tags and uses accumulated context as implicit bias for future recalls. Decoupled from plugin — these are memcp-side primitives usable by any caller.
- **Status**: DONE
- **Depends on**: Phase 08.9
- **Origin**: engram/.planning/memcp-knowledge-layer-vision.md (Phase 2 — Warm Recall, lines 113-124)
- **Plans:** 2/2 plans complete

Plans:
- [ ] 08.11-01-PLAN.md — Config extension (5 RecallConfig fields) + migration 019 (session_tags) + store methods + tag boost helpers + RecalledMemory extension [Wave 1]
- [ ] 08.11-02-PLAN.md — Wire boost into RecallEngine (both paths) + CLI --boost-tags + MCP boost_tags param + session tag accumulation [Wave 2, depends on 01]

## Phase 08.11.1: Bi-Temporal Search (event_time wiring)
- **Goal**: Wire `event_time` into temporal search boost. One-line fix in server.rs: prefer `event_time` over `created_at` when present in temporal range matching. Completes the bi-temporal search story started in Phase 08.8 (which added the schema + extraction but never wired it into retrieval). `let t = hit.memory.event_time.unwrap_or(hit.memory.created_at);`
- **Status**: Not started
- **Depends on**: Phase 08.11
- **Origin**: Competitive analysis (2026-03-03) — Zep/Graphiti bi-temporal comparison revealed memcp has the schema but doesn't query against event_time
- **Plans:** 1/1 plans complete

Plans:
- [ ] 08.11.1-01-PLAN.md — Wire event_time into temporal boost + unit test [Wave 1]

## Phase 08.12: HTTP API (Remote Daemon Mode)
- **Goal**: Extend the existing axum health server (port 9090) with API routes for core memcp operations. Add `--remote <url>` / `MEMCP_URL` env var to the CLI so it can route through HTTP instead of direct Postgres. Enables the OpenClaw plugin to call memcp over the network without Postgres credentials or a memcp binary in the OpenClaw container.
- **Status**: DONE
- **Depends on**: Phase 08.10
- **Origin**: engram Phase 3 Docker architecture — plugin needs to reach memcp from a separate container without shared Postgres credentials
- **Requirements:** [HTTP-01, HTTP-02, HTTP-03, HTTP-04, HTTP-05, HTTP-06, HTTP-07]
- **Plans:** 2/2 plans complete

Requirements:
- HTTP-01: `POST /v1/recall` — JSON API for recall (query-based and queryless), returns same shape as CLI `--json`
- HTTP-02: `POST /v1/search` — JSON API for hybrid search with all existing filter params
- HTTP-03: `POST /v1/store` — JSON API for memory storage with `wait: true` sync option
- HTTP-04: `POST /v1/annotate` + `POST /v1/update` — JSON API for memory modification
- HTTP-05: `GET /v1/status` alias + AppState expansion (HealthState -> AppState with config, embed_provider, embed_sender)
- HTTP-06: `--remote <url>` / `MEMCP_URL` global CLI flag — routes commands through HTTP instead of direct Postgres
- HTTP-07: CLI output in remote mode identical to local mode — transparent to callers

Plans:
- [ ] 08.12-01-PLAN.md — AppState + transport/api module (all 6 routes + handlers + types) + daemon wiring + integration tests [Wave 1]
- [ ] 08.12-02-PLAN.md — CLI --remote flag + dispatch_remote() helper + remote dispatch in 5 data commands + E2E tests [Wave 2, depends on 01]

**Why:** The memcp CLI currently connects directly to Postgres. In Docker (and any split deployment), this means every container that uses memcp needs the binary installed and DATABASE_URL with full DB credentials. An HTTP API on the existing daemon server lets callers reach memcp with just a URL. The daemon's connection pool handles Postgres access. Cleaner separation, no credential leakage, works for self-hosters on k8s or multi-machine setups.

**Routes (on existing axum server, port 9090):**

| Route | Maps to | Plugin use |
|-|-|-|
| `POST /v1/recall` | `cmd_recall` | Session start (queryless), warm recall, drift re-recall |
| `POST /v1/search` | `cmd_search` | Session bridge (find prior session summary) |
| `POST /v1/store` | `cmd_store` | Session summary, research auto-capture |
| `POST /v1/annotate` | `cmd_annotate` | Key moment enrichment |
| `POST /v1/update` | `cmd_update` | Project summary evolution |
| `GET /v1/status` | existing `/status` | Gateway health check (already exists, alias under /v1/) |

**Scope boundaries:**
- No auth on these routes yet (Phase 12 adds JWT). For now, routes are internal — Docker networking or localhost only.
- No MCP-over-HTTP. MCP serve stays stdio. This is a REST API for CLI-equivalent operations.
- No new capabilities — every route maps 1:1 to an existing CLI command.

## Phase 09: Documentation & QA Playbook
- **Goal**: README overhaul, config reference, architecture guide. Must cover both standalone and engram-bundled usage. Includes a scripted QA playbook (CLI + MCP flows against real Postgres) designed to be run by both humans and an AI QA agent.
- **Status**: Not planned
- **Depends on**: Phases 08.4, 08.5, 08.6, 08.7, 08.8
- **Note**: QA playbook absorbed from dropped Phase 08.1. Playbook should be structured for AI agent execution (machine-parseable assertions, clear pass/fail criteria) as well as human walkthroughs.

## Phase 10: Production Hardening
- **Goal**: Connection pool observability, global rate limiting on HTTP API, Prometheus metrics endpoint, and structured logging improvements. All work targets the daemon process (port 9090).
- **Status**: Planning complete
- **Depends on**: Phase 09
- **Requirements:** [PH-01, PH-02, PH-03, PH-04, PH-05, PH-06, PH-07]
- **Plans:** 3 plans
- **Note (quantum-safe encryption)**: TLS 1.3 with post-quantum key exchange (ML-KEM/Kyber) for DB connections. AES-256 at-rest via pgcrypto is already quantum-resistant. Consider optional column-level encryption for sensitive memory content. DEFERRED.

Requirements:
- PH-01: `GET /metrics` Prometheus scrape endpoint on port 9090 with all 13 declared metrics (request counters, duration histograms, pool gauges, worker counters)
- PH-02: Connection pool observability — poll pool.size()/num_idle() every 10s → Prometheus gauges, wire max_db_connections config into PgPoolOptions
- PH-03: Global rate limiting on `/v1/*` routes — per-endpoint token bucket via governor/tower_governor, configurable RPS, 429 with Retry-After + JSON body
- PH-04: Config structs — `RateLimitConfig` and `ObservabilityConfig` with serde defaults, `[rate_limit]` and `[observability]` sections in memcp.toml
- PH-05: Enriched `/status` endpoint — pool_active, pool_idle, pending embedding count, model name in component details
- PH-06: Structured logging — request-scoped tracing spans with request_id (UUID) + endpoint + method, `Redacted<T>` wrapper for memory content privacy
- PH-07: Worker metric instrumentation — GC runs/pruned counters, embedding jobs/duration, dedup merges, recall/search result count histograms

Plans:
- [ ] 10-01-PLAN.md — Prometheus infrastructure: dependencies, config structs, recorder install, /metrics endpoint, pool config wiring, pool poller [Wave 1]
- [ ] 10-02-PLAN.md — Rate limiting + metrics middleware on /v1/* routes + enriched /status [Wave 2, depends on 01]
- [ ] 10-03-PLAN.md — Structured logging (request spans, Redacted) + worker/handler metric instrumentation [Wave 2, depends on 01]

## Phase 10.1: Stress & Load Testing
- **Goal**: Load test all core operations under simulated multi-tenant conditions. Establish capacity numbers and known breaking points.
- **Status**: Not planned
- **Depends on**: Phase 10
- **Success Criteria**:
  1. Concurrent store/search benchmark at 10, 100, 1000 ops/sec
  2. Search latency vs corpus size (100, 1k, 10k, 100k memories)
  3. Embedding pipeline throughput under sustained load
  4. Memory/CPU profile per operation type
  5. Published capacity numbers for engram.host tier sizing

## Phase 11: System Review
- **Goal**: Codebase audit for quality/gaps before open-source release
- **Status**: Not planned
- **Depends on**: Phase 10.1

---
*Open-source fork cutoff: After Phase 11, fork memcp into a public MIT repo containing phases 01–11 (core memory server). Phase 12+ stays in the private memcp repo (or engram repo) — never published to the public fork. See engram Phase 4.5 and /Users/ayoamadi/projects/engram/.planning/ROADMAP.md for strategy.*

*Rationale: BSL doesn't prevent AI-assisted reimplementation in another language. Keeping competitive features in a private repo is stronger practical defense. Core memory server (01–11) is genuinely useful open-source; auth, boosting, and hosted features are the competitive moat.*

## Phase 12: Auth & API Keys
- **Goal**: API key authentication for the MCP interface. NOT full multi-tenant isolation inside memcp — engram uses container-per-tenant, so each memcp instance runs single-tenant. This phase adds the auth layer so a memcp instance rejects unauthorized callers.
- **Status**: Not planned
- **Depends on**: Phase 11 + public fork (Phase 12 code stays PRIVATE — never enters public memcp repo)
- **Note**: Full tenant isolation (row-level security, tenant IDs) is NOT needed. Container-per-tenant means isolation happens at the infrastructure level. This phase is about: API key validation, key rotation, and authenticated identity on stored memories.

## Phase 13: ~~claw-control API Surface~~
- **Status**: REMOVED — claw-control IS the dashboard. API endpoints will be added as needed by claw-control integration (engram Phase 4).
- **Note**: memcp does NOT need its own admin UI or dedicated API surface. claw-control calls memcp CLI/MCP directly.

## Phase 14: Memory Boosting (Competitor-Informed)
- **Goal**: Retrieval and evolution improvements informed by competitive landscape analysis (engram/.planning/competitive-landscape.md). Focuses on the highest-impact ideas from code review of 10+ competitor codebases.
- **Status**: Planned
- **Requirements:** [UUID-01, UUID-02, RET-01, RET-02, MQ-01, MQ-02, MQ-03, ENR-01, ENR-02, ENR-03, DISC-01, DISC-02, DISC-03]
- **Plans:** 5 plans

Requirements:
- UUID-01: UuidRefMap with session-scoped integer-to-UUID mapping
- UUID-02: All MCP tool responses include ref field, all ID inputs resolve integers
- RET-01: RetentionConfig with type_hint to FSRS stability mapping
- RET-02: store() applies type-specific initial stability
- MQ-01: DecomposedQuery type and decompose() trait method replacing expand()
- MQ-02: Ollama and OpenAI providers implement decompose()
- MQ-03: search_memory handler uses multi-query pipeline with rrf_fuse_multi()
- ENR-01: EnrichmentConfig and EnrichmentProvider trait
- ENR-02: Background sweep worker finding neighbors and suggesting tags via LLM
- ENR-03: Daemon wiring with config-gated startup
- DISC-01: discover_associations() cosine sweet-spot query in PostgresMemoryStore
- DISC-02: discover_memories MCP tool with LLM-generated connection explanations
- DISC-03: memcp discover CLI subcommand and POST /v1/discover HTTP API

Plans:
- [x] 14-01-PLAN.md — UUID hallucination prevention (integer ref mapping) [Wave 1]
- [ ] 14-02-PLAN.md — Type-specific retention via FSRS stability [Wave 1]
- [ ] 14-03-PLAN.md — Multi-query retrieval (decomposition + RRF fusion) [Wave 2]
- [ ] 14-04-PLAN.md — Retroactive neighbor enrichment (daemon worker) [Wave 2]
- [ ] 14-05-PLAN.md — Creative association discovery (CLI + MCP + API) [Wave 3]
- **Depends on**: Phase 12
- **Note**: PRIVATE — stays in private repo, never enters public memcp fork
- **Origin**: Competitive landscape research (2026-03-03) — Viren Mohindra's "State of Agent Memory 2026", SimpleMem, A-Mem, mcp-memory-service, Mem0

### Phase 14.1: Multi-Query Retrieval
- **Goal**: Decompose a search query into 1-4 targeted sub-queries, each hitting the hybrid search pipeline in parallel, then merge results. Modeled after SimpleMem's intent-aware retrieval planning — rated "the best retrieval strategy in the survey" by code review. Directly improves recall for complex/multi-faceted queries.
- **Status**: Planned
- **Depends on**: Phase 14
- **Source**: SimpleMem (arXiv:2601.02553)

### Phase 14.2: Type-Specific Retention Periods
- **Goal**: Make salience decay vary by memory type. Architecture decisions get longer retention (365 days), error observations get shorter (30 days), ephemeral context decays fastest. Uses existing `expires_at` column + `type_hint` to set retention at store time. Configurable retention schedule in memcp.toml.
- **Status**: Planned
- **Depends on**: Phase 14
- **Source**: mcp-memory-service (doobidoo)

### Phase 14.3: Retroactive Neighbor Enrichment
- **Goal**: When a new memory is stored, retrieve the 5 nearest existing memories and use an LLM to update their tags and context to reflect emerging patterns. New information doesn't just add to the store — it changes how old memories are represented. Addresses the "no feedback loop" gap. Makes the memory store compound over time.
- **Status**: Planned
- **Depends on**: Phase 14.1
- **Source**: A-Mem (NeurIPS 2025, arXiv:2502.12110, Zetzelkasten-inspired)

### Phase 14.4: Creative Association Discovery
- **Goal**: New query mode that searches the 0.3-0.7 cosine similarity "sweet spot" between memory pairs. Above 0.7 = redundant (already known to be related). Below 0.3 = noise. The sweet spot finds genuinely unexpected connections. Exposed as `memcp discover` CLI command and MCP tool.
- **Status**: Planned
- **Depends on**: Phase 14
- **Source**: mcp-memory-service ("dream-inspired" consolidation)

### Phase 14.5: UUID Hallucination Prevention
- **Goal**: Replace real UUIDs with integer indices before passing memory IDs to LLMs. Prevents a class of errors where models generate plausible-looking but invalid UUIDs. Transform layer in MCP tool responses and search result formatting.
- **Status**: Planned
- **Depends on**: Phase 14
- **Source**: Mem0 (discovered during code review)

### Phase 14.6: Standardized Benchmarking (LongMemEval + LoCoMo)
- **Goal**: Run memcp against LongMemEval and LoCoMo benchmarks. Publish scores. Every serious competitor publishes these — memcp is invisible without them. Extend existing Phase 06.3 benchmark harness with standard benchmark dataset runners. CI integration for regression.
- **Status**: In progress
- **Depends on**: Phase 12 (for public-facing numbers), but can run internally anytime
- **Source**: Competitive landscape analysis — table-stakes for credibility
- **Note**: Could be done pre-v1 for internal baseline. Public numbers published with open-source launch.
- **Requirements:** [BENCH-01, BENCH-02, BENCH-03, BENCH-04, BENCH-05, BENCH-06, BENCH-07, BENCH-08]
- **Plans:** 2/2 plans complete

Requirements:
- BENCH-01: LoCoMo dataset types (LoCoMoSample, Session, Turn, QaPair) with flexible category deserialization
- BENCH-02: LoCoMo dataset loader (locomo10.json parser)
- BENCH-03: SQuAD-style F1 scoring (token-level precision/recall/F1 with normalization)
- BENCH-04: LoCoMo runner with per-sample isolation (truncate, ingest conversation, evaluate all QA pairs)
- BENCH-05: Dual ingestion modes (per-turn and per-session) for LoCoMo conversations
- BENCH-06: Benchmark history tracking (JSONL append after each run with timestamp, scores, git SHA)
- BENCH-07: CLI --benchmark flag dispatching to LongMemEval or LoCoMo runners
- BENCH-08: CI workflow_dispatch for manual benchmark triggers

Plans:
- [ ] 14.6-01-PLAN.md — LoCoMo types, dataset loader, F1 scorer, ingestion, history append, judge model switch [Wave 1]
- [ ] 14.6-02-PLAN.md — LoCoMo runner, CLI dispatch extension, CI workflow [Wave 2, depends on 01]

## Phase 14.7: Benchmark Schema Isolation
- **Goal**: Postgres schema isolation for benchmark runs. Benchmarks create and use a dedicated `benchmark` schema instead of operating on `public`, preventing accidental data loss and enabling clean per-run isolation. Schema is ephemeral by default (dropped after run), with `--keep-schema` for post-run inspection.
- **Status**: Not planned
- **Depends on**: Phase 14.6
- **Note**: Uses Postgres `SET search_path` on connection pool — industry standard approach. No conflict with future Phase 12 multi-tenancy (RLS on public schema rows, orthogonal to schema isolation). Scope: benchmark binary only — tests stay on #[sqlx::test] ephemeral DBs.

## Phase 15: Import & Migration
- **Goal**: Import memories from external AI tools (OpenClaw, Claude Code, ChatGPT, Claude.ai, markdown, JSONL) and export memcp memories in multiple formats. The onboarding moment — user runs `memcp import`, instantly has thousands of searchable memories from existing AI usage. Three-tier curation pipeline (rule-based noise filter → optional LLM triage → existing memcp hygiene). Embedding reuse from OpenClaw for zero-cost import. Round-trip export for anti-lock-in.
- **Status**: DONE
- **Depends on**: Phase 08.12 (HTTP API for `--remote` import), Phase 08.4 (chunking for markdown import)
- **Design doc**: engram/.planning/memcp-import-design.md
- **Requirements:** [IMP-01, IMP-02, IMP-03, IMP-04, IMP-05, IMP-06, IMP-07, IMP-08, IMP-09, IMP-10, IMP-11, IMP-12]
- **Plans:** 1/1 plans complete

Requirements:
- IMP-01: `memcp import <source> [path]` CLI subcommand with 6 source readers (openclaw, claude-code, chatgpt, claude, markdown, jsonl)
- IMP-02: ImportSource trait + ImportEngine pipeline: read → noise filter → dedup → batch insert with progress bar
- IMP-03: Three-tier curation: Tier 1 rule-based noise filter (always), Tier 2 LLM triage (--curate), Tier 3 existing memcp hygiene (automatic)
- IMP-04: SHA-256 content-hash dedup within batch and against existing store; near-duplicates imported with `duplicate_of` tag
- IMP-05: Checkpoint/resume: interrupted imports resume from last completed batch; report.json with summary stats
- IMP-06: OpenClaw reader: SQLite chunks, memory→fact / sessions→observation, embedding reuse when model+dimension match
- IMP-07: Claude Code reader: MEMORY.md (fact, chunked by headers) + opt-in history.jsonl (observation)
- IMP-08: ChatGPT + Claude.ai + Markdown readers: ZIP parsing, per-conversation grouping, section-based chunking
- IMP-09: `memcp import --discover` auto-detects local sources, shows export instructions for non-local
- IMP-10: `memcp export --format <jsonl|csv|markdown>` with --output, --project, --tags, --since filters
- IMP-11: Export --include-embeddings and --include-state flags; JSONL round-trip fidelity with import
- IMP-12: `[import]` config section in memcp.toml for noise_patterns, batch_size, default_project

Plans:
- [x] 15-01-PLAN.md — Core import infrastructure: ImportSource trait, ImportEngine pipeline, noise filter, dedup, batch insert, checkpoint, progress bar, JSONL reader [Wave 1]
- [x] 15-02-PLAN.md — Export pipeline: JSONL/CSV/Markdown formatters, ExportEngine, CLI wiring, round-trip test [Wave 1]
- [x] 15-03-PLAN.md — OpenClaw reader (SQLite + embedding reuse) + Claude Code reader (MEMORY.md + history.jsonl) + Discovery command [Wave 2, depends on 01]
- [x] 15-04-PLAN.md — ChatGPT + Claude.ai ZIP readers + Markdown reader + Tier 2 LLM triage (--curate) [Wave 2, depends on 01]
- [x] 15-05-PLAN.md — Review/rescue commands + --remote import + ImportConfig + integration tests [Wave 3, depends on 03+04]
