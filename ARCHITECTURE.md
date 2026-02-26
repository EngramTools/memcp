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
│   │   │   └── benchmark/        Search quality benchmark harness
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
| store/postgres.rs | PostgreSQL implementation — CRUD, hybrid search (BM25 + pgvector), salience, embeddings, GC |

**Key types:** `MemoryStore` (trait), `Memory`, `CreateMemory`, `SearchFilter`, `SearchResult`, `PostgresMemoryStore`

### intelligence/ — Embedding, Search, Recall

| Module | Purpose |
|-|-|
| embedding/mod.rs | EmbeddingProvider trait, model dimension registry |
| embedding/local.rs | fastembed (All-MiniLM-L6-v2) — local CPU inference |
| embedding/openai.rs | OpenAI embedding API client |
| embedding/pipeline.rs | Async batch embedding processor (channel-based) |
| search/mod.rs | SalienceScorer — RRF fusion + FSRS-based salience adjustment |
| search/salience.rs | SalienceInput + scoring math (recency, access, reinforcement, stability) |
| recall/mod.rs | RecallEngine — session-aware context injection with dedup |
| query_intelligence/mod.rs | QueryIntelligenceProvider trait |
| query_intelligence/ollama.rs | Ollama LLM for query expansion + reranking |
| query_intelligence/openai.rs | OpenAI LLM for query expansion + reranking |
| query_intelligence/temporal.rs | Temporal hint parsing (e.g., "last week" -> date filter) |

**Key types:** `EmbeddingProvider` (trait), `EmbeddingPipeline`, `SalienceScorer`, `RecallEngine`, `QueryIntelligenceProvider` (trait)

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

**Key types:** `GcConfig`, `DedupWorker`, `ExtractionPipeline`, `ConsolidationWorker`, `AutoStoreWorker`, `CompositeFilter`

### transport/ — External Interfaces

| Module | Purpose |
|-|-|
| server.rs | MemoryService — rmcp MCP server with tool handlers (store, search, recall, etc.) |
| health/mod.rs | Axum HTTP server — /health (liveness) + /status (component health) |
| ipc.rs | Unix socket IPC — daemon embed/rerank requests from CLI |
| daemon.rs | Daemon orchestrator — spawns all workers, heartbeat loop, graceful shutdown |

**Key types:** `MemoryService`, `HealthState`, `run_daemon()`

## Data Flow

```
User/Agent -> CLI (main.rs) -> cmd_* handlers (cli.rs) -> MemoryStore (storage/)
                                                        -> IPC -> Daemon (embedding/rerank)

User/Agent -> MCP stdio -> MemoryService (transport/server.rs)
                           |-- store_memory -> ContentFilter -> MemoryStore -> EmbeddingPipeline
                           |-- search_memory -> EmbeddingProvider -> MemoryStore.hybrid_search -> SalienceScorer
                           |-- recall_memory -> RecallEngine -> EmbeddingProvider -> MemoryStore
                           '-- feedback_memory -> MemoryStore.apply_feedback

Daemon (transport/daemon.rs)
  |-- EmbeddingPipeline <- pending memories -> MemoryStore.update_embedding
  |-- ExtractionPipeline <- embedded memories -> MemoryStore.update_extraction
  |-- ConsolidationWorker <- extracted memories -> MemoryStore.consolidate
  |-- DedupWorker <- newly embedded -> find_similar -> merge_duplicate
  |-- GC Worker <- scheduled -> MemoryStore.gc_candidates -> soft_delete -> hard_purge
  |-- AutoStoreWorker <- JSONL files -> parse -> filter -> summarize -> MemoryStore.store
  |-- IPC Listener <- CLI embed/rerank requests -> EmbeddingProvider / QI Provider
  '-- Health Server <- HTTP probes -> MemoryStore.count_live_memories
```

## Configuration

All config loaded via `figment` (defaults -> `memcp.toml` -> environment variables):

| Config Struct | Section | Controls |
|-|-|-|
| `Config` | root | `database_url`, top-level settings |
| `EmbeddingConfig` | `[embedding]` | Provider, model, dimension |
| `SalienceConfig` | `[salience]` | FSRS parameters, scoring weights |
| `SearchConfig` | `[search]` | Default min_salience, field projection |
| `ExtractionConfig` | `[extraction]` | Enabled, provider, model |
| `ConsolidationConfig` | `[consolidation]` | Enabled, similarity threshold |
| `QueryIntelligenceConfig` | `[query_intelligence]` | Expansion/reranking providers |
| `GcConfig` | `[gc]` | Threshold, min age, TTL, purge grace |
| `DedupConfig` | `[dedup]` | Similarity threshold, enabled |
| `AutoStoreConfig` | `[auto_store]` | Watch paths, filter mode |
| `ContentFilterConfig` | `[content_filter]` | Regex patterns, excluded topics |
| `SummarizationConfig` | `[summarization]` | Enabled, provider, model |
| `RecallConfig` | `[recall]` | Max memories, min relevance, session TTL |
| `IdempotencyConfig` | `[idempotency]` | Dedup window, key TTL |
| `HealthConfig` | `[health]` | Port, bind address |
| `ResourceCapsConfig` | `[resource_caps]` | Max memories, max search results |

## Database

PostgreSQL with pgvector extension. Key tables:

- `memories` — core memory storage (content, metadata, provenance)
- `memory_embeddings` — vector embeddings (untyped vector column, HNSW index)
- `memory_salience` — FSRS salience state (stability, difficulty, retrievability)
- `memory_consolidations` — consolidation group tracking
- `daemon_status` — heartbeat + ingest metrics
- `sessions` / `session_recalls` — recall session tracking
- `idempotency_keys` — at-most-once store deduplication

Migrations managed by sqlx (compile-time checked, located at `crates/memcp-core/migrations/`).
