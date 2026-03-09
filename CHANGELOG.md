# Changelog

All notable changes to memcp are documented here, organized by phase.

## [0.1.0] - 2026-03-09

### Phase 11: System Review

- Zero-warning clippy build — hard errors enforced, logic bug fixed
- Removed stale feature flags (wave0_07_5, wave0_07_7)
- Resolved all pre-audit code quality issues
- MIT license, comprehensive README, updated CONTRIBUTING guide, deployment docs

### Phase 11.2: Trust-Weighted Retrieval & Curation Security

- Trust multiplier wired into composite and LLM re-ranking scoring
- Suspicious curation action type with quarantine mechanics
- Algorithmic instruction detection with trust-gated thresholds
- Priority queue scheduling for curation (P1/P2/Normal) with per-cluster LLM routing
- 20+ new tests for curation security and trust-weighted retrieval

### Phase 11.1: Provenance Tagging

- `trust_level`, `session_id`, `agent_role` columns added to memories table
- Transport-layer provenance: MCP, HTTP, and CLI sources tracked automatically
- Trust level inference from source and context at ingest time
- Auto-store session_id promotion and CLI provenance flags

### Phase 10: Production Hardening

- Prometheus metrics foundation: `AppState` registry, `/metrics` endpoint, pool config
- Per-endpoint rate limiting on all `/v1/*` routes with configurable RPS
- Metrics middleware with request duration histograms and status code labels
- Worker instrumentation: embedding, extraction, enrichment, curation, GC, discovery
- `/status` endpoint enriched with pool breakdown and embedding provider details
- Integration tests for rate limiting and metrics endpoints

### Phase 10.1: Stress & Load Testing

- Load test framework: corpus seeder, concurrent HTTP client driver, metrics collection
- CLI binary for running full test matrix with configurable concurrency and duration
- Report generation: JSON, Markdown, and baseline regression comparison
- Capacity report with P95/P99 latencies and throughput benchmarks

### Phase 10.2: Trust Workload Benchmark

- Poisoned corpus generator with templated injection patterns
- Trust distribution seeding with known-clean and known-poisoned memories
- Curation cycle runner with mock LLM provider for deterministic testing
- CLI `--profile trust` flag and `run_trust_workload_cli` orchestrator
- `SecurityReport` type with security correctness section and post-run audit
- End-to-end integration test for the full trust workload pipeline

### Phase 14: Memory Intelligence

- UUID ref mapping: hallucination prevention via short references in tool responses
- Type-specific retention: FSRS stability parameters per memory type
- Multi-query decomposition: complex queries split and results fused via RRF
- Retroactive neighbor enrichment: background sweep to enrich existing memories
- Creative association discovery: `discover_memories` tool finds cross-cluster associations

### Phase 08.6: Curation Dry-Run

- `--propose` flag for curation: shows proposed changes without applying them
- Dry-run output format for reviewing curation suggestions

### Phases 06.1–06.4: Search Enrichment

- **06.1**: Entity and fact extraction pipeline (Ollama + OpenAI providers), memory consolidation worker, three-way RRF fusion (BM25 + vector + symbolic)
- **06.2**: Query intelligence — expansion, re-ranking, and temporal hint parsing ("last week", "yesterday")
- **06.3**: Search quality benchmark harness, LoCoMo dataset ingestion, GPT-4o evaluation, benchmark CLI with CI integration
- **06.4**: `reinforce_memory` tool, implicit salience touch on `get_memory`

### Phase 06: Hybrid Search + Salience

- BM25 full-text search (PostgreSQL tsvector), GIN index
- pgvector HNSW index, semantic search with embedding similarity
- Hybrid search combining BM25 + vector results via RRF fusion
- FSRS-based salience scoring (recency, access frequency, reinforcement, stability)
- ParadeDB support as optional BM25 backend (`bm25_backend = "paradedb"`)

### Phase 05: Semantic Search

- fastembed local embedding inference (All-MiniLM-L6-v2, 384 dimensions)
- OpenAI embedding API client
- Async batch embedding pipeline (channel-based)
- `search_memory` tool with real vector similarity search

### Phase 04: Embedding Infrastructure

- `EmbeddingProvider` trait with local and OpenAI implementations
- Async embedding pipeline with background processing
- pgvector migration and `memory_embeddings` table

### Phase 03: PostgreSQL + Dev Infrastructure

- PostgreSQL backend replacing SQLite (pgvector-ready from the start)
- Docker Compose setup for local development
- GitHub Actions CI workflow
- Justfile with common dev commands

### Phase 02: MCP CRUD + Resources

- SQLite memory store with full CRUD operations
- All six MCP tools wired to real storage: `store_memory`, `get_memory`, `search_memory`, `list_memories`, `update_memory`, `delete_memory`
- MCP resource endpoints: `session-primer`, `user-profile`
- Comprehensive integration tests for full CRUD cycle

### Phase 01: Foundation

- Rust project scaffold (Tokio, rmcp, figment)
- MCP server with stdio transport
- Config, logging, and error infrastructure
- All six memory tools registered (initial stubs)
- MCP protocol integration tests
