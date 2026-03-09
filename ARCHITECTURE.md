# memcp Architecture

## Crate Structure

```
memcp/                            Cargo workspace root
├── crates/
│   ├── memcp-core/               Library crate (all logic)
│   │   ├── src/
│   │   │   ├── lib.rs            Crate root — domain module declarations + public API re-exports
│   │   │   ├── config.rs         Figment-based layered configuration
│   │   │   ├── errors.rs         MemcpError enum
│   │   │   ├── logging.rs        Tracing/subscriber initialization
│   │   │   ├── cli.rs            CLI subcommand handlers (cmd_store, cmd_search, etc.)
│   │   │   ├── storage/          Memory persistence layer
│   │   │   ├── intelligence/     Embedding, search, recall, query intelligence
│   │   │   ├── pipeline/         Background processing workers
│   │   │   ├── transport/        External interfaces (MCP, HTTP, IPC, daemon)
│   │   │   ├── benchmark/        Search quality benchmark harness
│   │   │   ├── load_test/        Stress and load testing framework
│   │   │   └── import/           Session log and file importers
│   │   ├── migrations/           PostgreSQL migrations (sqlx)
│   │   └── tests/                Integration + unit tests
│   └── memcp/                    Binary crate (thin CLI dispatcher)
│       └── src/main.rs           Clap CLI parsing → memcp-core entry points
```

## Domain Layers

### storage/ — Memory Persistence

| Module | Purpose |
|-|-|
| store/mod.rs | MemoryStore trait, Memory/CreateMemory/UpdateMemory types, SearchFilter, ListFilter, cursor encoding |
| store/postgres.rs | PostgreSQL implementation — CRUD, hybrid search (BM25 + pgvector), salience, embeddings, GC, provenance |

**Key types:** `MemoryStore` (trait), `Memory`, `CreateMemory`, `SearchFilter`, `SearchResult`, `PostgresMemoryStore`

**Provenance fields on `Memory`:** `trust_level` (f64, 0.0–1.0), `session_id` (Option<String>), `agent_role` (Option<String>), `source` (MCP/HTTP/CLI). Trust level is inferred from the transport layer at ingest time and participates in search scoring.

### intelligence/ — Embedding, Search, Recall

| Module | Purpose |
|-|-|
| embedding/mod.rs | EmbeddingProvider trait, model dimension registry |
| embedding/local.rs | fastembed (All-MiniLM-L6-v2) — local CPU inference |
| embedding/openai.rs | OpenAI embedding API client |
| embedding/pipeline.rs | Async batch embedding processor (channel-based) |
| search/mod.rs | SalienceScorer — RRF fusion + FSRS-based salience adjustment + trust weighting |
| search/salience.rs | SalienceInput + scoring math (recency, access, reinforcement, stability) |
| recall/mod.rs | RecallEngine — session-aware context injection with dedup |
| query_intelligence/mod.rs | QueryIntelligenceProvider trait |
| query_intelligence/ollama.rs | Ollama LLM for query expansion, re-ranking, and multi-query decomposition |
| query_intelligence/openai.rs | OpenAI LLM for query expansion, re-ranking, and multi-query decomposition |
| query_intelligence/temporal.rs | Temporal hint parsing (e.g., "last week" → date filter) |

**Key types:** `EmbeddingProvider` (trait), `EmbeddingPipeline`, `SalienceScorer`, `RecallEngine`, `QueryIntelligenceProvider` (trait)

**Trust-weighted scoring:** The `SalienceScorer` applies a `trust_level` multiplier when computing composite scores. Low-trust memories rank lower even if semantically relevant, providing a first line of defense against poisoned retrieval.

**Multi-query decomposition:** `QueryIntelligenceProvider` supports `decompose()` — splits a complex query into sub-queries, runs each independently, and fuses results via RRF. Useful for compound questions.

### pipeline/ — Background Workers

| Module | Purpose |
|-|-|
| gc/mod.rs | GcConfig, GcResult, GC candidate selection + soft-delete |
| gc/worker.rs | Background GC loop (salience threshold, TTL, hard purge) |
| gc/dedup.rs | DedupWorker — post-embedding semantic deduplication |
| extraction/mod.rs | ExtractionProvider trait |
| extraction/pipeline.rs | Async extraction processor (entities + facts via LLM) |
| extraction/ollama.rs | Ollama extraction provider |
| extraction/openai.rs | OpenAI extraction provider |
| consolidation/mod.rs | ConsolidationWorker — similarity-based memory merging |
| consolidation/similarity.rs | Cosine similarity helpers for consolidation |
| summarization/mod.rs | SummarizationProvider trait + factory |
| summarization/ollama.rs | Ollama summarization provider |
| summarization/openai.rs | OpenAI summarization provider |
| auto_store/mod.rs | AutoStoreWorker — JSONL session file watcher + ingester |
| auto_store/parser.rs | LogParser — Claude Code / OpenClaw session log parser |
| auto_store/filter.rs | CategoryFilter — heuristic content classification |
| auto_store/watcher.rs | Directory watcher for new session files |
| content_filter/mod.rs | ContentFilter trait, CompositeFilter (regex + semantic) |
| content_filter/regex_filter.rs | Regex pattern matching filter |
| content_filter/semantic_filter.rs | Embedding similarity topic exclusion |
| enrichment/mod.rs | EnrichmentProvider trait |
| enrichment/worker.rs | Background sweep for retroactive neighbor enrichment |
| curation/mod.rs | CurationProvider trait, cluster-based curation |
| curation/worker.rs | Background curation worker with priority queue (P1/P2/Normal) |
| curation/algorithmic.rs | Algorithmic instruction detection with trust-gated thresholds |
| curation/ollama.rs | Ollama LLM curation provider |
| curation/openai.rs | OpenAI LLM curation provider |
| promotion/mod.rs | Memory promotion (bump salience on recall hit) |
| temporal/mod.rs | Temporal decay and time-based salience adjustments |
| chunking/mod.rs | Long-content chunking for storage and retrieval |

**Key types:** `GcConfig`, `DedupWorker`, `ExtractionPipeline`, `ConsolidationWorker`, `AutoStoreWorker`, `CompositeFilter`, `CurationWorker`, `EnrichmentWorker`

**Curation security:** The `curation/` module detects and mitigates adversarial content in memory clusters. `algorithmic.rs` runs heuristic checks (instruction-like patterns, injection markers) before LLM review, gated by `trust_level`. Suspicious memories are quarantined (not purged) pending review. The priority queue ensures high-risk clusters (P1) are processed before routine curation (Normal).

### transport/ — External Interfaces

| Module | Purpose |
|-|-|
| server.rs | MemoryService — rmcp MCP server with tool handlers (store, search, recall, discover, etc.) |
| api/ | Axum HTTP API — `/v1/*` routes with rate limiting and request tracing |
| health/mod.rs | Axum HTTP server — /health (liveness) + /status (component health) |
| metrics.rs | Prometheus metrics registry, middleware, and `/metrics` endpoint |
| ipc.rs | Unix socket IPC — daemon embed/rerank requests from CLI |
| daemon.rs | Daemon orchestrator — spawns all workers, heartbeat loop, graceful shutdown |

**Key types:** `MemoryService`, `HealthState`, `AppState`, `run_daemon()`

**Provenance at transport boundaries:** Each transport layer (MCP, HTTP, CLI) sets `source` and infers `trust_level` before handing off to storage. MCP calls from local agents default to `trust_level = 0.8`; auto-store ingestion uses `trust_level = 0.3`; CLI stores use `trust_level = 1.0` by default (operator-sourced).

### benchmark/ — Search Quality Benchmarking

| Module | Purpose |
|-|-|
| mod.rs | Benchmark module root, types |
| dataset.rs | LoCoMo dataset parser and ingestion |
| evaluate.rs | GPT-4o evaluation pipeline |
| runner.rs | Per-question benchmark runner with configurable provider matrix |
| report.rs | Per-category metrics, Markdown/JSON report generation |
| prompts.rs | Evaluation prompt templates |
| locomo/ | LoCoMo dataset helpers |
| ingest.rs | Corpus ingestion for benchmark runs |

### load_test/ — Stress Testing Framework

| Module | Purpose |
|-|-|
| mod.rs | Load test types, configuration, result types |
| client.rs | Concurrent HTTP client driver (configurable workers, duration, endpoints) |
| corpus.rs | Corpus seeder with batch SQL inserts and random unit vectors |
| metrics.rs | Latency collection (P50/P95/P99), throughput, error rates |
| report.rs | Markdown + JSON reports, baseline regression comparison |
| trust.rs | Trust workload: poisoned corpus, curation cycle runner, security report |

### import/ — Data Importers

| Module | Purpose |
|-|-|
| mod.rs | Import entry points |
| (parsers) | Claude Code session logs, ChatGPT exports, Markdown files |

---

## Data Flow

```
User/Agent -> CLI (main.rs) -> cmd_* handlers (cli.rs) -> MemoryStore (storage/)
                                                        -> IPC -> Daemon (embedding/rerank)

User/Agent -> MCP stdio -> MemoryService (transport/server.rs)
                           |-- store_memory -> ContentFilter -> MemoryStore -> EmbeddingPipeline
                           |-- search_memory -> EmbeddingProvider -> MemoryStore.hybrid_search
                           |                   -> SalienceScorer (trust-weighted RRF fusion)
                           |-- recall_memory -> RecallEngine -> EmbeddingProvider -> MemoryStore
                           |-- discover_memories -> MemoryStore.discover_associations
                           '-- feedback_memory -> MemoryStore.apply_feedback

User/Agent -> HTTP API (transport/api/) -> rate limiter -> handler -> MemoryStore
                                          -> /v1/memories (CRUD + search)
                                          -> /metrics (Prometheus)
                                          -> /health + /status

Daemon (transport/daemon.rs)
  |-- EmbeddingPipeline <- pending memories -> MemoryStore.update_embedding
  |-- ExtractionPipeline <- embedded memories -> MemoryStore.update_extraction
  |-- ConsolidationWorker <- extracted memories -> MemoryStore.consolidate
  |-- DedupWorker <- newly embedded -> find_similar -> merge_duplicate
  |-- EnrichmentWorker <- existing memories -> retroactive neighbor enrichment
  |-- CurationWorker <- memory clusters -> algorithmic check -> LLM review -> quarantine/approve
  |-- GC Worker <- scheduled -> MemoryStore.gc_candidates -> soft_delete -> hard_purge
  |-- AutoStoreWorker <- JSONL files -> parse -> filter -> summarize -> MemoryStore.store
  |-- IPC Listener <- CLI embed/rerank requests -> EmbeddingProvider / QI Provider
  '-- Health Server <- HTTP probes -> MemoryStore.count_live_memories
```

---

## Configuration

All config loaded via `figment` (defaults → `memcp.toml` → environment variables with `MEMCP__` prefix):

| Config Struct | Section | Controls |
|-|-|-|
| `Config` | root | `database_url`, top-level settings |
| `EmbeddingConfig` | `[embedding]` | Provider, model, dimension |
| `SalienceConfig` | `[salience]` | FSRS parameters, scoring weights |
| `SearchConfig` | `[search]` | Default min_salience, BM25 backend, salience hint mode |
| `ExtractionConfig` | `[extraction]` | Enabled, provider, model |
| `ConsolidationConfig` | `[consolidation]` | Enabled, similarity threshold |
| `QueryIntelligenceConfig` | `[query_intelligence]` | Expansion/reranking/decomposition providers |
| `GcConfig` | `[gc]` | Threshold, min age, TTL, purge grace |
| `DedupConfig` | `[dedup]` | Similarity threshold, enabled |
| `AutoStoreConfig` | `[auto_store]` | Watch paths, filter mode |
| `ContentFilterConfig` | `[content_filter]` | Regex patterns, excluded topics |
| `SummarizationConfig` | `[summarization]` | Enabled, provider, model |
| `RecallConfig` | `[recall]` | Max memories, min relevance, session TTL |
| `IdempotencyConfig` | `[idempotency]` | Dedup window, key TTL |
| `HealthConfig` | `[health]` | Port, bind address |
| `ResourceCapsConfig` | `[resource_caps]` | Max memories, max search results |
| `RetentionConfig` | `[retention]` | Type-specific FSRS stability parameters |
| `MetricsConfig` | `[metrics]` | Prometheus endpoint, scrape settings |
| `RateLimitConfig` | `[rate_limit]` | Per-endpoint RPS limits |

---

## Database

PostgreSQL with pgvector extension. Key tables:

- `memories` — core memory storage (content, metadata, provenance: trust_level, session_id, agent_role, source)
- `memory_embeddings` — vector embeddings (untyped vector column, HNSW index)
- `memory_salience` — FSRS salience state (stability, difficulty, retrievability)
- `memory_consolidations` — consolidation group tracking
- `memory_uuid_refs` — short UUID reference mapping for hallucination prevention
- `daemon_status` — heartbeat + ingest metrics
- `sessions` / `session_recalls` — recall session tracking
- `idempotency_keys` — at-most-once store deduplication

Migrations managed by sqlx (compile-time checked, located at `crates/memcp-core/migrations/`).

---

## Key Design Decisions

**Trust-weighted retrieval:** Trust level is a first-class property of every memory, set at ingest time based on source. It participates in composite scoring so adversarial or low-confidence memories naturally rank lower without explicit filtering.

**Curation as defense-in-depth:** Background curation workers review memory clusters for adversarial content. Algorithmic pre-screening (heuristic injection detection) runs before LLM review to reduce cost. Suspicious memories are quarantined, not deleted — preserving audit trails.

**Agent-first perspective:** Tool schema descriptions are the primary discovery mechanism for agents. Parameters are designed so agents use them correctly without system prompt hints. Both MCP and HTTP paths produce identical results for the same inputs.

**Local-first by default:** fastembed provides CPU-only embedding inference with no API key or network requirement. All LLM-dependent features (extraction, summarization, query intelligence) are opt-in and disabled by default.
