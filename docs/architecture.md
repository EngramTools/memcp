# memcp Architecture

## Crate Structure

```
memcp/                            Cargo workspace root
├── crates/
│   ├── memcp-core/               Library crate (all logic)
│   │   ├── src/
│   │   │   ├── lib.rs            Crate root, module declarations, public API re-exports
│   │   │   ├── config.rs         Figment-based layered configuration
│   │   │   ├── errors.rs         MemcpError enum
│   │   │   ├── logging.rs        Tracing/subscriber initialization
│   │   │   ├── cli.rs            CLI subcommand handlers
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
│       └── src/main.rs           Clap CLI parsing -> memcp-core entry points
```

## Domain Layers

### storage/ -- Memory Persistence

| Module | Purpose |
|-|-|
| store/mod.rs | MemoryStore trait, Memory/CreateMemory/UpdateMemory types, SearchFilter, ListFilter, cursor encoding |
| store/postgres.rs | PostgreSQL implementation -- CRUD, hybrid search (BM25 + pgvector), salience, embeddings, GC, provenance |

Key types: `MemoryStore` (trait), `Memory`, `CreateMemory`, `SearchFilter`, `SearchResult`, `PostgresMemoryStore`

Provenance fields on `Memory`: `trust_level` (f64, 0.0-1.0), `session_id`, `agent_role`, `source` (MCP/HTTP/CLI). Trust level is inferred from the transport layer at ingest time and participates in search scoring.

### intelligence/ -- Embedding, Search, Recall

| Module | Purpose |
|-|-|
| embedding/mod.rs | EmbeddingProvider trait, model dimension registry |
| embedding/local.rs | fastembed (All-MiniLM-L6-v2) -- local CPU inference |
| embedding/openai.rs | OpenAI embedding API client |
| embedding/pipeline.rs | Async batch embedding processor (channel-based) |
| search/mod.rs | SalienceScorer -- RRF fusion + FSRS-based salience adjustment + trust weighting |
| search/salience.rs | SalienceInput + scoring math (recency, access, reinforcement, stability) |
| recall/mod.rs | RecallEngine -- session-aware context injection with dedup |
| query_intelligence/mod.rs | QueryIntelligenceProvider trait |
| query_intelligence/ollama.rs | Ollama LLM for query expansion, re-ranking, multi-query decomposition |
| query_intelligence/openai.rs | OpenAI LLM for query expansion, re-ranking, multi-query decomposition |
| query_intelligence/temporal.rs | Temporal hint parsing (e.g., "last week" -> date filter) |

Key types: `EmbeddingProvider` (trait), `EmbeddingPipeline`, `SalienceScorer`, `RecallEngine`, `QueryIntelligenceProvider` (trait)

Trust-weighted scoring: `SalienceScorer` applies a `trust_level` multiplier when computing composite scores. Low-trust memories rank lower even if semantically relevant.

Multi-query decomposition: `QueryIntelligenceProvider` supports `decompose()` -- splits complex queries into sub-queries, runs each independently, fuses results via RRF.

### pipeline/ -- Background Workers

| Module | Purpose |
|-|-|
| gc/mod.rs | GcConfig, GcResult, GC candidate selection + soft-delete |
| gc/worker.rs | Background GC loop (salience threshold, TTL, hard purge) |
| gc/dedup.rs | DedupWorker -- post-embedding semantic deduplication |
| extraction/mod.rs | ExtractionProvider trait |
| extraction/pipeline.rs | Async extraction processor (entities + facts via LLM) |
| extraction/ollama.rs | Ollama extraction provider |
| extraction/openai.rs | OpenAI extraction provider |
| consolidation/mod.rs | ConsolidationWorker -- similarity-based memory merging |
| consolidation/similarity.rs | Cosine similarity helpers for consolidation |
| summarization/mod.rs | SummarizationProvider trait + factory |
| summarization/ollama.rs | Ollama summarization provider |
| summarization/openai.rs | OpenAI summarization provider |
| auto_store/mod.rs | AutoStoreWorker -- JSONL session file watcher + ingester |
| auto_store/parser.rs | LogParser -- Claude Code / OpenClaw session log parser |
| auto_store/filter.rs | CategoryFilter -- heuristic content classification |
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
| redaction/mod.rs | RedactionEngine -- two-phase regex scan for secrets and PII |

Key types: `GcConfig`, `DedupWorker`, `ExtractionPipeline`, `ConsolidationWorker`, `AutoStoreWorker`, `CompositeFilter`, `CurationWorker`, `EnrichmentWorker`, `RedactionEngine`

### transport/ -- External Interfaces

| Module | Purpose |
|-|-|
| server.rs | MemoryService -- rmcp MCP server with tool handlers (store, search, recall, discover, etc.) |
| api/ | Axum HTTP API -- `/v1/*` routes with rate limiting and request tracing |
| health/mod.rs | Axum HTTP server -- /health (liveness) + /status (component health) |
| metrics.rs | Prometheus metrics registry, middleware, and `/metrics` endpoint |
| ipc.rs | Unix socket IPC -- daemon embed/rerank requests from CLI |
| daemon.rs | Daemon orchestrator -- spawns all workers, heartbeat loop, graceful shutdown |

Key types: `MemoryService`, `HealthState`, `AppState`, `run_daemon()`

Provenance at transport boundaries: each transport (MCP, HTTP, CLI) sets `source` and infers `trust_level` before handing off to storage. MCP calls default to `trust_level = 0.8`; auto-store ingestion uses `0.3`; CLI stores use `1.0` (operator-sourced).

## Key Subsystems

### Ingestion Pipeline

```
store request
  -> redaction (secret detection, PII masking)
  -> content filter (regex patterns, semantic topic exclusion)
  -> idempotency check (content hash dedup)
  -> write to memories table (provenance: source, trust_level)
  -> embedding pipeline (fastembed or OpenAI, async)
  -> post-embed: dedup worker (near-duplicate merge)
  -> post-embed: extraction (entity + fact extraction via LLM)
  -> chunking (long content split into overlapping sentence groups)
```

### Retrieval Pipeline

```
search request
  -> temporal hint parsing ("last week" -> date filter)
  -> query expansion (LLM-based synonym + context expansion)
  -> multi-query decomposition (complex queries split into sub-queries)
  -> hybrid search: BM25 full-text + pgvector cosine similarity
  -> RRF fusion (reciprocal rank fusion across search signals)
  -> salience scoring (recency, access frequency, reinforcement, semantic relevance)
  -> trust weighting (trust_level multiplier on composite scores)
  -> LLM re-ranking (optional, final precision pass)
```

### Curation Pipeline

```
curation worker (periodic, background)
  -> cluster memories by embedding similarity
  -> instruction detection (algorithmic heuristics, trust-gated thresholds)
  -> priority queue: P1 (adversarial) > P2 (suspicious) > Normal
  -> LLM review (optional, for flagged clusters)
  -> actions: merge related, strengthen important, flag stale, quarantine adversarial
```

## Data Flow

```
User/Agent -> CLI (main.rs) -> cmd_* handlers (cli.rs) -> MemoryStore (storage/)
                                                        -> IPC -> Daemon (embedding/rerank)

User/Agent -> MCP stdio -> MemoryService (transport/server.rs)
                           |-- store_memory -> Redaction -> ContentFilter -> MemoryStore -> EmbeddingPipeline
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

## Database

PostgreSQL with pgvector extension. Key tables:

- `memories` -- core storage (content, metadata, provenance: trust_level, session_id, agent_role, source)
- `memory_embeddings` -- vector embeddings (untyped vector column, HNSW index)
- `memory_salience` -- FSRS salience state (stability, difficulty, retrievability)
- `memory_consolidations` -- consolidation group tracking
- `memory_uuid_refs` -- short UUID reference mapping for hallucination prevention
- `daemon_status` -- heartbeat + ingest metrics
- `sessions` / `session_recalls` -- recall session tracking
- `idempotency_keys` -- at-most-once store deduplication

Migrations managed by sqlx (compile-time checked, located at `crates/memcp-core/migrations/`).

## Key Design Decisions

**Trust-weighted retrieval:** Trust level is a first-class property of every memory, set at ingest time based on source. It participates in composite scoring so adversarial or low-confidence memories naturally rank lower without explicit filtering.

**Curation as defense-in-depth:** Background curation workers review memory clusters for adversarial content. Algorithmic pre-screening (heuristic injection detection) runs before LLM review to reduce cost. Suspicious memories are quarantined, not deleted, preserving audit trails.

**Agent-first perspective:** Tool schema descriptions are the primary discovery mechanism for agents. Parameters are designed so agents use them correctly without system prompt hints. Both MCP and HTTP paths produce identical results for the same inputs.

**Local-first by default:** fastembed provides CPU-only embedding inference with no API key or network requirement. All LLM-dependent features (extraction, summarization, query intelligence, curation) are opt-in and disabled by default.

**Redaction on ingestion:** Secrets (API keys, tokens, private keys) are detected and masked before storage. PII detection is opt-in. Allowlists and custom rules provide escape hatches. Fail-open: redaction errors never block storage.
