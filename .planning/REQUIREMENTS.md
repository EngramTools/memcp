# Requirements Traceability

**Generated:** 2026-03-10
**Source:** Collated from `.planning/ROADMAP.md` (definitions), `*-PLAN.md` (assignments), `*-SUMMARY.md` (completion evidence)

## Summary Statistics

| Metric | Count |
|-|-|
| Total REQ-IDs (ROADMAP definitions) | 120 |
| REQ-IDs with SUMMARY completion evidence | ~80 unique |
| REQ-IDs in PLAN frontmatter | ~108 unique |
| Status DONE (SUMMARY evidence) | 83 |
| Status PLANNED (PLAN assigned, no SUMMARY) | 25 |
| Status UNTRACKED (ROADMAP only) | 12 |
| Additional IDs (PLAN-only, no ROADMAP definition) | 16 |

---

## Traceability Table

### CLI -- CLI Interface (Phase 06.5)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| CLI-01 | `memcp store` -- write memory to DB with `embedding_status='pending'`, exit immediately | 06.5 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| CLI-02 | `memcp search` -- hybrid search with salience scoring, JSON output | 06.5 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| CLI-03 | `memcp list` -- list/filter memories with pagination | 06.5 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| CLI-04 | `memcp get` / `memcp delete` / `memcp reinforce` -- single-memory operations | 06.5 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| CLI-05 | `memcp daemon` -- long-running process hosting embedding pipeline, extraction pipeline, consolidation worker, and auto-store sidecar | 06.5 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| CLI-06 | `memcp --help` -- concise usage guide suitable for single-read agent consumption | 06.5 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| CLI-07 | Trim MCP tool descriptions (shorter param docs, move guidance to instructions field) | 06.5 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| CLI-08 | CLAUDE.md / agent config instructions for CLI-first usage | 06.5 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| CLI-09 | Remove memcp from MCP server config after CLI is validated | 06.5 | -- | DONE | Phase completed (pre-requirements-completed convention) |

### SUM -- Auto-Summarization (Phase 06.6)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| SUM-01 | Auto-store sidecar summarizes AI assistant responses before storing (configurable: on/off, provider, model) | 06.6 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| SUM-02 | Raw unsummarized storage still available -- agent can store directly via CLI/MCP without summarization | 06.6 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| SUM-03 | Provenance distinguishes summarized vs raw: `type_hint: "summary"` + tag `"summarized"` for summaries, original `type_hint` preserved for raw | 06.6 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| SUM-04 | Summarization uses existing Ollama/OpenAI provider infrastructure (zero API cost with local Ollama) | 06.6 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| SUM-05 | Configurable summarization prompt template (what to extract/compress from AI responses) | 06.6 | -- | DONE | Phase completed (pre-requirements-completed convention) |

### MOD -- Modularity (Phase 07)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| MOD-01 | `fastembed` is optional behind `local-embed` Cargo feature (default on). `--no-default-features` builds without it. | 07 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| MOD-02 | Embedding model and dimension configurable via `memcp.toml` | 07 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| MOD-03 | Model dimension registry maps known model names to their output dimensions | 07 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| MOD-04 | Database schema supports any vector dimension (untyped `vector` column, migration removes `vector(384)` constraint) | 07 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| MOD-05 | Daemon creates HNSW index at startup matching configured dimension | 07 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| MOD-06 | `memcp embed switch-model` handles dimension changes: detects mismatch, purges incompatible embeddings, drops/recreates HNSW index | 07 | -- | DONE | Phase completed (pre-requirements-completed convention) |

### AS -- Auto-Store Claude Code Integration (Phase 07.1)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| AS-01 | Auto-store sidecar watches a directory and picks up new `.jsonl` files as they appear (glob/directory watching support) | 07.1 | 07.1-01 | DONE | 07.1-01-SUMMARY |
| AS-02 | Default `filter_mode` changed from `"llm"` to `"none"` -- works out of the box without Ollama; users opt in to LLM filtering via config | 07.1 | 07.1-01 | DONE | 07.1-01-SUMMARY |
| AS-03 | `memcp search` CLI gains `--source` flag for filtering by memory source | 07.1 | 07.1-01 | DONE | 07.1-01-SUMMARY |

### SSI -- Sidecar Status Indicator (Phase 07.3)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| SSI-01 | `memcp status` JSON output enriched with sidecar metrics and model info | 07.3 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| SSI-02 | `memcp status --pretty` -- human-readable one-liner showing daemon state | 07.3 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| SSI-03 | `memcp status --check` -- deep health check probing DB, Ollama, model cache, and watch paths | 07.3 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| SSI-04 | Auto-store worker writes `last_ingest_at` to daemon_status after each successful memory store | 07.3 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| SSI-05 | Claude Code status line script showing daemon health indicator | 07.3 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| SSI-06 | `memcp statusline install` copies script to `~/.claude/scripts/` and prints settings.json instructions | 07.3 | -- | DONE | Phase completed (pre-requirements-completed convention) |

### HYG -- Memory Hygiene (Phase 07.4)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| HYG-01 | Salience-threshold GC -- background worker prunes memories below configurable salience threshold after configurable age | 07.4 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| HYG-02 | Optional TTL support -- memories can have an `expires_at` timestamp; GC worker deletes expired memories | 07.4 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| HYG-03 | Semantic deduplication on ingest -- embedding similarity check against recent memories; if above threshold, merge or skip | 07.4 | 07.4-03 | DONE | 07.4-03-SUMMARY |
| HYG-04 | Category-aware auto-store filtering -- classify incoming content and skip low-value categories | 07.4 | 07.4-02 | DONE | 07.4-02-SUMMARY |
| HYG-05 | GC dry-run mode -- `memcp gc --dry-run` shows what would be pruned without deleting | 07.4 | -- | DONE | Phase completed (pre-requirements-completed convention) |
| HYG-06 | GC metrics in `memcp status` -- show last GC run, items pruned, dedup merges | 07.4 | -- | DONE | Phase completed (pre-requirements-completed convention) |

### SCF -- Search Consistency & Feedback (Phase 07.5)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| SCF-01 | CLI search uses the same hybrid search pipeline as MCP serve | 07.5 | 07.5-00, 07.5-01, 07.5-04 | DONE | 07.5-00-SUMMARY, 07.5-04-SUMMARY |
| SCF-02 | `memcp feedback <id> useful\|irrelevant` -- explicit relevance signal that adjusts FSRS stability/difficulty | 07.5 | 07.5-00, 07.5-02 | DONE | 07.5-00-SUMMARY |
| SCF-03 | Search results include a feedback hint (memory ID) so agents can easily provide feedback | 07.5 | 07.5-00, 07.5-01 | DONE | 07.5-00-SUMMARY |
| SCF-04 | Cursor-based pagination for search and list -- keyset pagination | 07.5 | 07.5-00, 07.5-03 | DONE | 07.5-00-SUMMARY, 07.5-03-SUMMARY |
| SCF-05 | Deprecate offset-based pagination with migration path | 07.5 | 07.5-00, 07.5-03 | DONE | 07.5-00-SUMMARY, 07.5-03-SUMMARY |

### CEX -- Code Execution Support (Phase 07.6)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| CEX-01 | `search_memory` gains `fields` param -- field projection to return only requested fields | 07.6 | 07.6-01 | DONE | 07.6-01-SUMMARY |
| CEX-02 | `search_memory` gains `min_salience` param -- server-side salience threshold filtering | 07.6 | 07.6-01 | DONE | 07.6-01-SUMMARY |
| CEX-03 | `allowed_callers` annotation on MCP tool definitions -- mark tools as callable from code execution | 07.6 | 07.6-02 | DONE | 07.6-02-SUMMARY |
| CEX-04 | Tool descriptions include structured output format documentation (JSON schema of return types) | 07.6 | 07.6-02 | DONE | 07.6-02-SUMMARY |

### IDP -- Idempotent Tool Operations (Phase 07.7)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| IDP-01 | `store_memory` deduplicates on content hash -- if identical content exists within time window, return existing ID | 07.7 | 07.7-01 | DONE | Phase completed (pre-requirements-completed convention) |
| IDP-02 | Optional `idempotency_key` param on `store_memory` -- caller-provided key for at-most-once storage | 07.7 | 07.7-01 | DONE | Phase completed (pre-requirements-completed convention) |
| IDP-03 | `search_memory` and `delete_memory` are naturally idempotent -- document this contract in tool descriptions | 07.7 | 07.7-01 | DONE | Phase completed (pre-requirements-completed convention) |

### RCL -- Recall (Phase 07.11)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| RCL-01 | `memcp recall` CLI + MCP tool -- query-based recall with session dedup | 07.11 | 07.11-02 | DONE | Phase completed (pre-requirements-completed convention) |
| RCL-02 | `session_recalls` table (migration) -- session_id, memory_id FK, recalled_at, relevance | 07.11 | 07.11-01 | DONE | Phase completed (pre-requirements-completed convention) |
| RCL-03 | Configurable cap and relevance threshold via `recall.max_memories` and `recall.min_relevance` | 07.11 | 07.11-01 | DONE | Phase completed (pre-requirements-completed convention) |
| RCL-04 | Tiered recall strategy -- extraction enabled: search `extracted_facts`; disabled: filter to fact/summary type_hints | 07.11 | 07.11-02 | DONE | Phase completed (pre-requirements-completed convention) |
| RCL-05 | Compaction-aware dedup -- `recall` accepts optional `reset=true` param to clear session recall history | 07.11 | 07.11-02 | DONE | Phase completed (pre-requirements-completed convention) |
| RCL-06 | Implicit salience reinforcement -- each recall bumps FSRS stability for the recalled memory | 07.11 | 07.11-01, 07.11-02 | DONE | Phase completed (pre-requirements-completed convention) |

### TST -- Testing (Phase 08)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| TST-01 | All 12 inline `#[cfg(test)]` modules migrated from src/ to tests/unit/ | 08 | 08-01 | DONE | 08-01-SUMMARY |
| TST-02 | Test infrastructure: tests/common/ with MemoryBuilder, golden dataset loader, shared helpers | 08 | 08-01 | DONE | 08-01-SUMMARY |
| TST-03 | Integration tests for uncovered modules: recall, gc/dedup, feedback, summarization, embedding pipeline | 08 | 08-02 | DONE | 08-02-SUMMARY |
| TST-04 | E2E journey tests: store -> embed -> search -> recall -> salience decay -> reinforcement | 08 | 08-03 | DONE | 08-03-SUMMARY |
| TST-05 | MCP contract tests using McpTestClient: all MCP tool operations tested via stdio protocol | 08 | 08-03 | DONE | 08-03-SUMMARY |
| TST-06 | Golden dataset for search quality regression: fixture file with known queries -> expected results | 08 | 08-04 | DONE | 08-04-SUMMARY |
| TST-07 | Auto-store sidecar E2E: simulated JSONL conversation input -> verify memories ingested correctly | 08 | 08-03 | DONE | 08-03-SUMMARY |
| TST-08 | CI coverage enforcement: cargo-llvm-cov with threshold, GitHub Actions integration | 08 | 08-05 | PLANNED | -- |

### CL -- Container Lifecycle (Phase 08.2)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| CL-01 | `/health` HTTP endpoint (200/503 liveness) and `/status` endpoint (component breakdown) | 08.2 | 08.2-01 | DONE | Phase completed (pre-requirements-completed convention) |
| CL-02 | SIGTERM triggers graceful shutdown: reject new requests, flush pending embeddings, close DB connections | 08.2 | 08.2-03 | DONE | 08.2-03-SUMMARY |
| CL-03 | Configurable resource caps (max_memories, max_embedding_batch_size, max_search_results, max_db_connections) | 08.2 | 08.2-02 | DONE | 08.2-02-SUMMARY |
| CL-04 | Startup readiness with DB connection retry (exponential backoff up to 30s) | 08.2 | 08.2-01, 08.2-03 | DONE | 08.2-03-SUMMARY |

### HTTP -- HTTP API / Remote Daemon Mode (Phase 08.12)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| HTTP-01 | `POST /v1/recall` -- JSON API for recall (query-based and queryless) | 08.12 | 08.12-01 | DONE | Phase completed (pre-requirements-completed convention) |
| HTTP-02 | `POST /v1/search` -- JSON API for hybrid search with all existing filter params | 08.12 | 08.12-01 | DONE | Phase completed (pre-requirements-completed convention) |
| HTTP-03 | `POST /v1/store` -- JSON API for memory storage with `wait: true` sync option | 08.12 | 08.12-01 | DONE | Phase completed (pre-requirements-completed convention) |
| HTTP-04 | `POST /v1/annotate` + `POST /v1/update` -- JSON API for memory modification | 08.12 | 08.12-01 | DONE | Phase completed (pre-requirements-completed convention) |
| HTTP-05 | `GET /v1/status` alias + AppState expansion (HealthState -> AppState) | 08.12 | 08.12-01 | DONE | Phase completed (pre-requirements-completed convention) |
| HTTP-06 | `--remote <url>` / `MEMCP_URL` global CLI flag -- routes commands through HTTP instead of direct Postgres | 08.12 | 08.12-02 | DONE | Phase completed (pre-requirements-completed convention) |
| HTTP-07 | CLI output in remote mode identical to local mode -- transparent to callers | 08.12 | 08.12-02 | DONE | Phase completed (pre-requirements-completed convention) |

### PH -- Production Hardening (Phase 10)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| PH-01 | `GET /metrics` Prometheus scrape endpoint on port 9090 with all 13 declared metrics | 10 | 10-01, 10-05 | DONE | 10-01-SUMMARY |
| PH-02 | Connection pool observability -- pool.size()/num_idle() Prometheus gauges, max_db_connections wiring | 10 | 10-01 | DONE | 10-01-SUMMARY |
| PH-03 | Global rate limiting on `/v1/*` routes -- per-endpoint token bucket, configurable RPS, 429 with Retry-After | 10 | 10-02, 10-04, 10-05 | PLANNED | -- |
| PH-04 | Config structs -- `RateLimitConfig` and `ObservabilityConfig` with serde defaults | 10 | 10-01 | DONE | 10-01-SUMMARY |
| PH-05 | Enriched `/status` endpoint -- pool_active, pool_idle, pending embedding count, model name | 10 | 10-02 | PLANNED | -- |
| PH-06 | Structured logging -- request-scoped tracing spans with request_id + endpoint + method, `Redacted<T>` wrapper | 10 | 10-02 | PLANNED | -- |
| PH-07 | Worker metric instrumentation -- GC runs/pruned counters, embedding jobs/duration, dedup merges | 10 | 10-03, 10-04, 10-05 | DONE | 10-03-SUMMARY |

### PROV -- Provenance Tagging (Phase 06.4 + Phase 11.1)

Phase 06.4 references PROV-01, PROV-02 in its Requirements array. Phase 11.1 defines PROV-01 through PROV-10 with a broader scope (trust_level, session_id, agent_role metadata). The PROV-01/02 in Phase 06.4 refer to basic actor/actor_type provenance; Phase 11.1's PROV-01/02 add trust_level and OWASP ASI06 defense. See Anomaly #4 for details.

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| PROV-01 | Trust_level + session_id + agent_role metadata on every memory write (Phase 11.1 scope) | 06.4, 11.1 | 11.1-01 | DONE | 11.1-01-SUMMARY |
| PROV-02 | Provenance defense against memory poisoning (OWASP ASI06) | 06.4, 11.1 | 11.1-01 | DONE | 11.1-01-SUMMARY |
| PROV-03 | (Phase 11.1 scope -- no ROADMAP definition) | 11.1 | 11.1-01 | DONE | 11.1-01-SUMMARY |
| PROV-04 | (Phase 11.1 scope -- no ROADMAP definition) | 11.1 | 11.1-01 | DONE | 11.1-01-SUMMARY |
| PROV-05 | (Phase 11.1 scope -- no ROADMAP definition) | 11.1 | 11.1-01 | DONE | 11.1-01-SUMMARY |
| PROV-06 | (Phase 11.1 scope -- no ROADMAP definition) | 11.1 | 11.1-01 | DONE | 11.1-01-SUMMARY |
| PROV-07 | (Phase 11.1 scope -- no ROADMAP definition) | 11.1 | 11.1-02 | DONE | 11.1-02-SUMMARY |
| PROV-08 | (Phase 11.1 scope -- no ROADMAP definition) | 11.1 | 11.1-02 | DONE | 11.1-02-SUMMARY |
| PROV-09 | (Phase 11.1 scope -- no ROADMAP definition) | 11.1 | 11.1-02 | DONE | 11.1-02-SUMMARY |
| PROV-10 | (Phase 11.1 scope -- no ROADMAP definition) | 11.1 | 11.1-02 | DONE | 11.1-02-SUMMARY |

### EXCL -- Topic Exclusion (Phase 06.4)

Listed in Phase 06.4 Requirements array `[EXCL-01..EXCL-06]` but no formal `- EXCL-NN: description` lines exist in ROADMAP.

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| EXCL-01 | (No ROADMAP definition -- Phase 06.4 topic exclusion) | 06.4 | -- | UNTRACKED | -- |
| EXCL-02 | (No ROADMAP definition -- Phase 06.4 topic exclusion) | 06.4 | -- | UNTRACKED | -- |
| EXCL-03 | (No ROADMAP definition -- Phase 06.4 topic exclusion) | 06.4 | -- | UNTRACKED | -- |
| EXCL-04 | (No ROADMAP definition -- Phase 06.4 topic exclusion) | 06.4 | -- | UNTRACKED | -- |
| EXCL-05 | (No ROADMAP definition -- Phase 06.4 topic exclusion) | 06.4 | -- | UNTRACKED | -- |
| EXCL-06 | (No ROADMAP definition -- Phase 06.4 topic exclusion) | 06.4 | -- | UNTRACKED | -- |

### TWR -- Trust-Weighted Retrieval & Curation Security (Phase 11.2 + Phase 11)

TWR-01 through TWR-08 are defined in ROADMAP under Phase 11.2-11.3 and also listed in Phase 11 (System Review) Requirements. Phase 11.2 implemented them first; Phase 11 re-verified/fixed them during system review.

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| TWR-01 | Trust multiplier in composite scoring -- score = 0.5 * RRF + 0.5 * (salience * trust_level) | 11.2, 11 | 11.2-01, 11-01 | DONE | 11.2-01-SUMMARY, 11-01-SUMMARY |
| TWR-02 | Trust multiplier in LLM re-ranking -- 0.7 * llm_rank + 0.3 * (salience * trust) | 11.2, 11 | 11.2-01, 11-02 | DONE | 11.2-01-SUMMARY, 11-02-SUMMARY |
| TWR-03 | Suspicious curation action -- quarantine with tag + trust=0.05 + audit trail | 11.2, 11 | 11.2-02, 11-01 | DONE | 11.2-02-SUMMARY, 11-01-SUMMARY |
| TWR-04 | Quarantined memories excluded from all search via skip_tags | 11.2, 11 | 11.2-02, 11-04 | DONE | 11.2-02-SUMMARY, 11-04-SUMMARY |
| TWR-05 | Un-quarantine restores previous trust_level from trust_history | 11.2, 11 | 11.2-02, 11-01 | DONE | 11.2-02-SUMMARY, 11-01-SUMMARY |
| TWR-06 | Algorithmic instruction detection with trust-gated thresholds (1/2/3 signals) | 11.2, 11 | 11.2-02, 11-01 | DONE | 11.2-02-SUMMARY, 11-01-SUMMARY |
| TWR-07 | LLM prompt instruction-detection dimension + parse suspicious action | 11.2, 11 | 11.2-03, 11-03 | DONE | 11.2-03-SUMMARY |
| TWR-08 | Priority queue ordering -- P1 (low trust+new) before P2 before Normal | 11.2, 11 | 11.2-03, 11-04 | DONE | 11.2-03-SUMMARY, 11-04-SUMMARY |

### TRUST -- Load Test Trust Features (Phase 10.2)

TRUST-01 through TRUST-08 are defined in Phase 10.2's RESEARCH.md and VALIDATION.md but have no formal ROADMAP definition lines (Phase 10.2 was inserted as urgent work with `Requirements: TBD`).

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| TRUST-01 | Load test for trust-weighted retrieval | 10.2 | 10.2-01 | DONE | 10.2-01-SUMMARY |
| TRUST-02 | Load test for trust-weighted retrieval (scoring) | 10.2 | 10.2-01 | DONE | 10.2-01-SUMMARY |
| TRUST-03 | Load test for quarantine functionality | 10.2 | 10.2-02 | DONE | 10.2-02-SUMMARY |
| TRUST-04 | Load test for quarantine exclusion | 10.2 | 10.2-02 | DONE | 10.2-02-SUMMARY |
| TRUST-05 | Load test for curation features | 10.2 | 10.2-02 | DONE | 10.2-02-SUMMARY |
| TRUST-06 | Security report section renders correctly | 10.2 | 10.2-03 | DONE | 10.2-VERIFICATION (no SUMMARY for 10.2-03) |
| TRUST-07 | CLI --profile trust flag | 10.2 | 10.2-03 | DONE | 10.2-VERIFICATION (no SUMMARY for 10.2-03) |
| TRUST-08 | Full trust workload e2e | 10.2 | 10.2-03 | DONE | 10.2-VERIFICATION (no SUMMARY for 10.2-03) |

### UUID/RET/MQ/ENR/DISC -- Memory Boosting (Phase 14)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| UUID-01 | UuidRefMap with session-scoped integer-to-UUID mapping | 14 | 14-01 | DONE | 14-01-SUMMARY |
| UUID-02 | All MCP tool responses include ref field, all ID inputs resolve integers | 14 | 14-01 | DONE | 14-01-SUMMARY |
| RET-01 | RetentionConfig with type_hint to FSRS stability mapping | 14 | 14-02 | DONE | Phase completed (no SUMMARY completion evidence, but phase marked DONE) |
| RET-02 | store() applies type-specific initial stability | 14 | 14-02 | DONE | Phase completed (no SUMMARY completion evidence, but phase marked DONE) |
| MQ-01 | DecomposedQuery type and decompose() trait method replacing expand() | 14 | 14-03 | DONE | 14-03-SUMMARY |
| MQ-02 | Ollama and OpenAI providers implement decompose() | 14 | 14-03 | DONE | 14-03-SUMMARY |
| MQ-03 | search_memory handler uses multi-query pipeline with rrf_fuse_multi() | 14 | 14-03 | DONE | 14-03-SUMMARY |
| ENR-01 | EnrichmentConfig and EnrichmentProvider trait | 14 | 14-04 | DONE | Phase completed (no SUMMARY completion evidence, but phase marked DONE) |
| ENR-02 | Background sweep worker finding neighbors and suggesting tags via LLM | 14 | 14-04 | DONE | Phase completed (no SUMMARY completion evidence, but phase marked DONE) |
| ENR-03 | Daemon wiring with config-gated startup | 14 | 14-04 | DONE | Phase completed (no SUMMARY completion evidence, but phase marked DONE) |
| DISC-01 | discover_associations() cosine sweet-spot query in PostgresMemoryStore | 14 | 14-05 | DONE | Phase completed (no SUMMARY completion evidence, but phase marked DONE) |
| DISC-02 | discover_memories MCP tool with LLM-generated connection explanations | 14 | 14-05 | DONE | Phase completed (no SUMMARY completion evidence, but phase marked DONE) |
| DISC-03 | memcp discover CLI subcommand and POST /v1/discover HTTP API | 14 | 14-05 | DONE | Phase completed (no SUMMARY completion evidence, but phase marked DONE) |

### BENCH -- Standardized Benchmarking (Phase 14.6)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| BENCH-01 | LoCoMo dataset types (LoCoMoSample, Session, Turn, QaPair) with flexible category deserialization | 14.6 | 14.6-01 | DONE | 14.6-01-SUMMARY |
| BENCH-02 | LoCoMo dataset loader (locomo10.json parser) | 14.6 | 14.6-01 | DONE | 14.6-01-SUMMARY |
| BENCH-03 | SQuAD-style F1 scoring (token-level precision/recall/F1 with normalization) | 14.6 | 14.6-01 | DONE | 14.6-01-SUMMARY |
| BENCH-04 | LoCoMo runner with per-sample isolation (truncate, ingest conversation, evaluate all QA pairs) | 14.6 | 14.6-02 | DONE | 14.6-02-SUMMARY |
| BENCH-05 | Dual ingestion modes (per-turn and per-session) for LoCoMo conversations | 14.6 | 14.6-01 | DONE | 14.6-01-SUMMARY |
| BENCH-06 | Benchmark history tracking (JSONL append after each run with timestamp, scores, git SHA) | 14.6 | 14.6-01 | DONE | 14.6-01-SUMMARY |
| BENCH-07 | CLI --benchmark flag dispatching to LongMemEval or LoCoMo runners | 14.6 | 14.6-02 | DONE | 14.6-02-SUMMARY |
| BENCH-08 | CI workflow_dispatch for manual benchmark triggers | 14.6 | 14.6-02 | DONE | 14.6-02-SUMMARY |

### IMP -- Import & Migration (Phase 15)

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| IMP-01 | `memcp import <source> [path]` CLI subcommand with 6 source readers | 15 | 15-01 | PLANNED | -- |
| IMP-02 | ImportSource trait + ImportEngine pipeline: read -> noise filter -> dedup -> batch insert | 15 | 15-01 | PLANNED | -- |
| IMP-03 | Three-tier curation: Tier 1 rule-based noise filter, Tier 2 LLM triage, Tier 3 memcp hygiene | 15 | 15-01, 15-05 | PLANNED | -- |
| IMP-04 | SHA-256 content-hash dedup within batch and against existing store | 15 | 15-01, 15-05 | PLANNED | -- |
| IMP-05 | Checkpoint/resume: interrupted imports resume from last completed batch | 15 | 15-01, 15-05 | PLANNED | -- |
| IMP-06 | OpenClaw reader: SQLite chunks, memory->fact / sessions->observation, embedding reuse | 15 | 15-03 | DONE | 15-03-SUMMARY |
| IMP-07 | Claude Code reader: MEMORY.md (fact, chunked by headers) + opt-in history.jsonl | 15 | 15-03 | DONE | 15-03-SUMMARY |
| IMP-08 | ChatGPT + Claude.ai + Markdown readers: ZIP parsing, per-conversation grouping | 15 | 15-04 | PLANNED | -- |
| IMP-09 | `memcp import --discover` auto-detects local sources, shows export instructions for non-local | 15 | 15-01, 15-05 | PLANNED | -- |
| IMP-10 | `memcp export --format <jsonl\|csv\|markdown>` with --output, --project, --tags, --since filters | 15 | 15-02 | DONE | 15-02-SUMMARY |
| IMP-11 | Export --include-embeddings and --include-state flags; JSONL round-trip fidelity | 15 | 15-02 | DONE | 15-02-SUMMARY |
| IMP-12 | `[import]` config section in memcp.toml for noise_patterns, batch_size, default_project | 15 | 15-05 | PLANNED | -- |

### BENCH-SAFE -- Benchmark Safety Hardening (Phase 18)

BENCH-SAFE-01 through BENCH-SAFE-04 are referenced in ROADMAP Requirements arrays for Phases 17, 18, 19, and 20 (copy-paste artifact). No formal `- BENCH-SAFE-NN: description` lines exist. Assigned to Phase 18 based on SUMMARY completion evidence.

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| BENCH-SAFE-01 | Schema-gated truncate_all() with safety guards | 18 | 18-01 | DONE | 18-01-SUMMARY |
| BENCH-SAFE-02 | --destructive flag requirement for benchmark runner | 18 | 18-01 | DONE | 18-01-SUMMARY |
| BENCH-SAFE-03 | Production URL detection and warning | 18 | 18-01 | DONE | 18-01-SUMMARY |
| BENCH-SAFE-04 | Load test binary safety hardening (--destructive flag + URL check) | 18 | 18-02 | DONE | 18-02-SUMMARY |

### TWR-RECALL -- Trust-Weighted Recall (Phase 17)

TWR-RECALL-01 through TWR-RECALL-04 appear in Phase 17 PLAN/SUMMARY frontmatter. Phase 17's ROADMAP Requirements array erroneously lists `[BENCH-SAFE-01..04]` (copy-paste from Phase 18). The actual requirements are TWR-RECALL-*.

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| TWR-RECALL-01 | Wire trust_level into recall scoring path | 17 | 17-01 | DONE | 17-01-SUMMARY |
| TWR-RECALL-02 | Wire trust_level into recall LLM re-ranking | 17 | 17-01 | DONE | 17-01-SUMMARY |
| TWR-RECALL-03 | Trust-weighted recall integration tests | 17 | 17-01 | DONE | 17-01-SUMMARY |
| TWR-RECALL-04 | Recall path trust scoring validation | 17 | 17-01 | DONE | 17-01-SUMMARY |

### RT -- Requirements Traceability (Phase 19)

Referenced in Phase 19 ROADMAP Requirements array but no formal definition lines.

| REQ-ID | Description | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|-|
| RT-01 | Master requirements traceability table | 19 | 19-01 | DONE | 19-01-SUMMARY |
| RT-02 | Anomalies section documenting data quality issues | 19 | 19-01 | DONE | 19-01-SUMMARY |
| RT-03 | Pre-REQ-ID phases acknowledged | 19 | 19-01 | DONE | 19-01-SUMMARY |

---

## PLAN-Only IDs (No ROADMAP Definition)

These REQ-IDs appear in PLAN frontmatter but have no corresponding `- PREFIX-NN: description` line in ROADMAP.md and are not listed in any ROADMAP Requirements array.

| REQ-ID | Phase | Plan(s) | Status | Evidence | Notes |
|-|-|-|-|-|-|
| STOR-01 | 03 | 03-01 | DONE | Phase completed | Early phase, pre-convention |
| STOR-02 | 03 | 03-01 | DONE | Phase completed | Early phase, pre-convention |
| STOR-04 | 06 | 06-01 | DONE | Phase completed | Early phase, pre-convention |
| INFR-01 | 03 | 03-02 | DONE | Phase completed | Early phase, pre-convention |
| INFR-02 | 03 | 03-03 | DONE | Phase completed | Early phase, pre-convention |
| SRCH-01 | 06 | 06-01, 06-03 | DONE | Phase completed | Early phase, pre-convention |
| SRCH-02 | 06 | 06-03 | DONE | Phase completed | Early phase, pre-convention |
| SRCH-03 | 06 | 06-02 | DONE | Phase completed | Early phase, pre-convention |
| SRCH-04 | 06 | 06-02, 06-04 | DONE | Phase completed | Early phase, pre-convention |
| SRCH-05 | 06 | 06-02, 06-04 | DONE | Phase completed | Early phase, pre-convention |
| SRCH-06 | 06 | 06-03, 06-04 | DONE | Phase completed | Early phase, pre-convention |
| EMBD-01 | 04 | 04-01, 04-02 | DONE | 04-01-SUMMARY | Early phase |
| EMBD-02 | 04 | 04-01 | DONE | 04-01-SUMMARY | Early phase |
| EMBD-03 | 04 | 04-01 | DONE | 04-01-SUMMARY | Early phase |
| EMBD-04 | 04 | 04-02 | DONE | Phase completed | In PLAN but not in ROADMAP definitions |
| ENRCH-01 | 06.1 | 06.1-01 | DONE | Phase completed | Early phase, pre-convention |
| ENRCH-02 | 06.1 | 06.1-03 | DONE | 06.1-03-SUMMARY | Early phase |
| ENRCH-03 | 06.1 | 06.1-02 | DONE | Phase completed | Early phase, pre-convention |
| TDB-BUILDER | 07.2 | 07.2-02 | DONE | 07.2-02-SUMMARY | Test database, ad-hoc ID |
| TDB-STRESS | 07.2 | 07.2-03 | DONE | 07.2-03-SUMMARY | Test database, ad-hoc ID |
| TDB-MCP-LIFECYCLE | 07.2 | 07.2-04 | PLANNED | -- | Test database, ad-hoc ID |
| TDB-MCP-CONTRACT | 07.2 | 07.2-04 | PLANNED | -- | Test database, ad-hoc ID |

### Ad-Hoc IDs from Phase 16 (Audit Priorities)

These reference test coverage audit priorities from AUDIT.md, not formal ROADMAP requirements.

| REQ-ID | Phase | Plan(s) | Status | Evidence |
|-|-|-|-|-|
| P1-1 | 16 | 16-01 | DONE | 16-01-SUMMARY |
| P1-1b | 16 | 16-01 | DONE | 16-01-SUMMARY |
| P2-6 | 16 | 16-02 | DONE | 16-02-SUMMARY |
| P2-7 | 16 | 16-01 | DONE | 16-01-SUMMARY |

---

## Pre-REQ-ID Phases

The following phases were completed before formal REQ-ID tracking was established. They delivered real capabilities but predate the convention. ROADMAP marks them as DONE.

| Phase | Name | Status |
|-|-|-|
| 01 | Foundation | DONE |
| 02 | Core Memory | DONE |
| 03 | Persistence | DONE |
| 04 | Embeddings | DONE |
| 05 | Vector Search | DONE |
| 06 | Hybrid Search + Salience | DONE |
| 06.1 | Search Enrichment | DONE |
| 06.2 | Query Intelligence | DONE |
| 06.3 | Memory Benchmarking | DONE |
| 06.4 | Provenance + Topic Exclusion | DONE |

Additional phases completed without formal REQ-ID tracking (despite being created after the convention began):

| Phase | Name | Status | Notes |
|-|-|-|-|
| 07.2 | Test Database | DONE | Used ad-hoc IDs (TDB-*) |
| 08.1 | Regression Suite & Manual QA | DONE | No requirements defined |
| 08.3 | Modularize | DONE | No requirements defined |
| 08.4 | Memory Chunking | DONE | No requirements defined |
| 08.5 | API & Pipeline Polish | DONE | No requirements defined |
| 08.6 | AI Brain Curation | DONE | No requirements defined |
| 08.7 | Multi-Model Embeddings | DONE | No requirements defined |
| 08.8 | Plugin Support Primitives | DONE | No requirements defined |
| 08.9 | Query-less Recall | DONE | No requirements defined |
| 08.10 | Memory Content Updates | DONE | No requirements defined |
| 08.11 | Warm Recall & Session-Aware Ranking | DONE | No requirements defined |
| 08.11.1 | Bi-Temporal Search | DONE | No requirements defined |
| 09 | Documentation & QA Playbook | DONE | No requirements defined |
| 10.1 | Stress & Load Testing | DONE | No requirements defined |
| 14.7 | Benchmark Schema Isolation | DONE | No requirements defined |

---

## Anomalies

### 1. Orphaned SUMMARY IDs

**TDB-BUILDER** and **TDB-STRESS** appear in SUMMARY `requirements-completed:` fields (07.2-02-SUMMARY, 07.2-03-SUMMARY) but have no ROADMAP definition. These are ad-hoc IDs for Phase 07.2 (Test Database) which was inserted without formal requirements.

### 2. Ad-Hoc Audit Priority IDs

**P1-1**, **P1-1b**, **P2-6**, **P2-7** appear in Phase 16 SUMMARYs. These reference test coverage audit priority items from AUDIT.md (e.g., "P1-1: salience rank() unit test"), not formal ROADMAP requirements. They follow a different naming convention (priority-based rather than subsystem-based).

### 3. BENCH-SAFE-* Duplicated Across Phases 17-20 in ROADMAP

BENCH-SAFE-01 through BENCH-SAFE-04 are listed in the `Requirements:` array of four different phases in ROADMAP.md:
- Phase 17: Trust-Weighted Recall -- **incorrect** (should be TWR-RECALL-*)
- Phase 18: Benchmark Safety Hardening -- **correct** (SUMMARY evidence confirms completion here)
- Phase 19: Requirements Traceability -- **incorrect** (should be RT-*)
- Phase 20: Test Quality Fixes -- **incorrect** (should have its own IDs)

This is a copy-paste artifact from when Phases 17-20 were batch-created. BENCH-SAFE-* is assigned to Phase 18 in this table based on SUMMARY completion evidence.

### 4. PROV-01/02 Split Ownership Between Phase 06.4 and Phase 11.1

Phase 06.4 (Provenance + Topic Exclusion) lists `[PROV-01, PROV-02, EXCL-01..06]` in its Requirements array. Phase 11.1 (Provenance Tagging) lists `[PROV-01..PROV-10]`. The scope differs:
- **Phase 06.4 PROV-01/02**: Basic actor/actor_type provenance columns
- **Phase 11.1 PROV-01/02**: Expanded trust_level + session_id + agent_role (OWASP ASI06)

Neither phase has formal `- PROV-NN: description` definition lines in ROADMAP. PROV-01 through PROV-10 completion evidence comes from Phase 11.1 SUMMARYs.

### 5. SCF-01 Progressive Completion

SCF-01 appears in `requirements-completed:` for both 07.5-00-SUMMARY and 07.5-04-SUMMARY, and is assigned in PLANs 07.5-00, 07.5-01, and 07.5-04. This reflects progressive completion -- the search consistency feature was delivered across multiple plans within Phase 07.5.

### 6. EMBD-04 in PLAN but Missing from ROADMAP

EMBD-04 appears in Phase 04's 04-02-PLAN.md `requirements:` frontmatter but has no `- EMBD-04: description` definition line in ROADMAP.md. The embedding phase (04) predates formal requirement definitions. EMBD-01/02/03 also lack ROADMAP definition lines but do appear in 04-01-SUMMARY completion evidence.

### 7. TWR-* Duplicated Between Phase 11.2 and Phase 11

TWR-01 through TWR-08 are defined under Phase 11.2-11.3 in ROADMAP and also listed in Phase 11 (System Review) Requirements array. Both phases have SUMMARY completion evidence:
- **Phase 11.2**: Original implementation (trust-weighted retrieval + curation security)
- **Phase 11**: System review pass that re-verified and fixed TWR requirements

This is intentional -- Phase 11 was designed to audit Phase 11.2's work. The table lists both phases as evidence sources.
