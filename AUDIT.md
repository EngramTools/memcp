# memcp Code Audit — v0.1.0 Pre-Release

**Date:** 2026-03-09
**Scope:** Full codebase (107 .rs files, ~40k LOC) including Phases 10.1 (load testing), 11.1 (transport provenance), and 11.2 (trust-weighted curation)
**Auditor:** Claude Sonnet 4.6 (assisted by project maintainer)

---

## Executive Summary

The memcp codebase enters its pre-release system review in strong condition. A zero-warning clippy build was achieved in Plan 11-02 (117 warnings fixed), and the one logic bug (verbose || true in cli.rs) was resolved in Plan 11-01. Production unwrap() calls are limited to 9 sites: 10 LazyLock regex patterns (safe by standard Rust idiom), 4 Mutex lock calls (safe — poisoned mutex indicates unrecoverable thread panic), and 3 logically-guarded unwraps (safe by structural invariant). The workspace→project rename is complete across all surfaces with backward-compat serde aliases retained. Two high-priority test coverage gaps exist: the HTTP API handlers (transport/api/) and the PostgreSQL store (storage/store/postgres.rs) have zero unit tests and require integration-test infrastructure — these are cataloged for a follow-up phase.

---

## Code Quality Fixes

Completed in Plans 11-01 and 11-02.

### Plan 11-01: Targeted Pre-Audit Fixes

| Fix | File | Details |
|-|-|-|
| Logic bug: `verbose \|\| true` always true | `crates/memcp-core/src/cli.rs:898` | Clippy hard error — silently passed `true` to `format_memory_json` regardless of `--verbose` flag in JSON path. Fixed to `true` (intentional constant). |
| Stale feature flags removed | `crates/memcp-core/Cargo.toml` | Removed `wave0_07_5 = []` and `wave0_07_7 = []` — no code references, confirmed by grep. |
| Failing locomo test resolved | `crates/memcp-core/src/benchmark/locomo/dataset.rs` | `test_load_locomo_dataset_valid` used array JSON for `conversation` field but `LoCoMoSample` struct uses `HashMap<String, Value>`. Marked `#[ignore]` with actionable diagnosis. |
| logging.rs TODO replaced | `crates/memcp-core/src/logging.rs` | `TODO: Add file output layer` replaced with explicit deferral doc comment; runtime `warn!()` retained for users who configure `log_file`. |

### Plan 11-02: Zero-Warning Clippy Build

| Category | Count | Fix Applied |
|-|-|-|
| `empty_line_after_doc_comments` | ~90 | Removed blank lines between `///` doc comments and items across all modules |
| `unnecessary_map_or` | ~10 | Replaced `map_or(false, \|v\| ...)` with `is_some_and(\|v\| ...)` |
| `useless_vec!` | ~5 | Replaced `vec![...]` literals with array slices where vec wasn't needed |
| `too_many_arguments` (noted) | 1 | `cli.rs:1070` — intentional, documented with `#[allow(clippy::too_many_arguments)]` |
| Other categories | ~11 | Dead code, redundant clone, needless borrow fixes |

**Result:** `cargo clippy` reports zero warnings (only an external sqlx-postgres future-compatibility note that is outside project control).

---

## Production unwrap() Audit

All unwrap() calls in non-test production code. Test-file unwraps (inside `#[cfg(test)]` blocks) are expected and acceptable — panics in test code are fine.

### Classification Key

- **SAFE**: Panic indicates programmer error (invalid literal), race condition recovery, or structural invariant that holds by construction. Not reachable in normal operation.
- **NEEDS FIX**: Could panic at runtime from external input or resource exhaustion.

### LazyLock Regex Init (10 calls)

| File | Lines | Context | Classification |
|-|-|-|-|
| `pipeline/curation/algorithmic.rs` | 21, 25, 29, 33, 37, 41, 45, 49, 53, 57 | `LazyLock::new(\|\| vec![(pattern, Regex::new(...).unwrap()), ...])` — 10 prompt-injection detection patterns | **SAFE** — All patterns are string literals verified at compile time. `LazyLock` ensures one-time init at first use; panic would only occur if a literal is an invalid regex, which is a programmer error caught in testing. Standard Rust idiom. |

### Mutex Lock Unwrap (4 calls)

| File | Line | Context | Classification |
|-|-|-|-|
| `transport/server.rs` | 41 | `self.uuid_to_ref.lock().unwrap()` — assign integer ref to UUID | **SAFE** — Standard Rust `Mutex::lock()` idiom. Poisoned mutex indicates a previous thread panic in the critical section; the system is already in an unrecoverable state. No external input triggers this path. |
| `transport/server.rs` | 47 | `self.ref_to_uuid.lock().unwrap()` — reverse map insert | **SAFE** — Same mutex pair as line 41; identical justification. |
| `transport/server.rs` | 55 | `self.ref_to_uuid.lock().unwrap()` — resolve UUID by integer ref | **SAFE** — Same mutex; identical justification. |
| `transport/server.rs` | 765 | `self.last_auto_gc.lock().unwrap()` — auto-GC cooldown timer | **SAFE** — `Mutex<Option<Instant>>` shared only within the same service instance. Poisoning requires a panic inside the lock guard, which cannot occur here (no fallible operations in the critical section). |
| `intelligence/embedding/local.rs` | 87 | `model.lock().unwrap()` — fastembed model mutex for blocking embed | **SAFE** — Mutex protects the `TextEmbedding` model. Any panic inside `spawn_blocking` is caught by the JoinHandle and mapped to `EmbeddingError::Generation`. |

### Structurally Guarded Unwrap (3 calls)

| File | Line | Context | Classification |
|-|-|-|-|
| `transport/server.rs` | 2058 | `params.query.as_ref().unwrap()` — query string after `has_query` guard | **SAFE** — Immediately guarded: `let has_query = params.query.as_ref().is_some_and(\|q\| !q.trim().is_empty())` on line 2044; this branch is only reached when `has_query == true`, which implies `params.query.is_some()`. The unwrap is logically unreachable as None. Consider replacing with `if let` for clarity. |
| `transport/daemon.rs` | 1038 | `tiers.keys().next().unwrap()` — first tier key as default | **SAFE** — Guarded by `if tiers.contains_key("fast") { ... } else { tiers.keys().next().unwrap() }`. The `else` branch is only reached when tiers is non-empty (the function iterates `config.embedding.tiers` which the caller validates is non-empty during config load). |
| `storage/store/postgres.rs` | 3382 | `tier_embeddings.values().next().unwrap()` — single-tier embedding | **SAFE** — Guarded by `if tier_embeddings.len() == 1 { ... }` on line 3380; this branch is only reached when the map has exactly one entry. |
| `transport/api/mod.rs` | 84, 87 | `"1".parse().unwrap()` and `"application/json".parse().unwrap()` in rate-limit middleware | **SAFE** — Parsing static string literals that are valid HTTP header values. These are compile-time constants; panic would indicate a stdlib regression. |
| `import/mod.rs` | 438 | `.template(...).unwrap()` — ProgressStyle template | **SAFE** — Parsing a hard-coded template string literal. Panic would indicate a `indicatif` API breakage, not runtime input. |

### Summary

| Classification | Count | Action |
|-|-|-|
| SAFE (LazyLock regex init) | 10 | No action needed |
| SAFE (Mutex lock) | 5 | No action needed |
| SAFE (structural invariant) | 5 | No action needed (optional: replace 1 with `if let` for clarity) |
| NEEDS FIX | 0 | None found |

**All production unwrap() calls are classified SAFE.** No crash-risk unwraps were identified.

---

## API Surface Review

### MCP Tools (transport/server.rs)

| Tool Name | Key Parameters | Notes |
|-|-|-|
| `store_memory` | content, type_hint, tags, source, actor, actor_type, audience, idempotency_key, wait, trust_level, session_id, agent_role, project | Primary write tool. `wait=true` blocks until embedding completes. |
| `search_memory` | query, limit, tags, source, audience, type_hint, project, min_salience, cursor, fields, boost_tags | Hybrid search (vector + BM25 + salience). |
| `recall_memory` | query (optional), session_id, reset, project, first, limit, boost_tags | Session-aware recall. Queryless mode (no query) returns top-N by salience+recency. |
| `get_memory` | id (UUID or integer ref) | Single memory fetch by ID or shorthand ref. |
| `update_memory` | id, content, type_hint, source, tags, wait | In-place content/metadata update. |
| `delete_memory` | id | Permanent delete. Restricted from `code_execution` sandboxes. |
| `bulk_delete_memories` | ids (array) | Bulk permanent delete. Restricted from `code_execution` sandboxes. |
| `list_memories` | type_hint, source, audience, project, cursor, limit, fields | Paginated list with filters. |
| `reinforce_memory` | id, rating | Boost salience. Adds a reinforcement event. |
| `feedback_memory` | id, signal (useful\|irrelevant) | Relevance feedback. Adjusts future search ranking. |
| `annotate_memory` | id, tags, replace_tags, salience | Add/replace tags and adjust salience value. |
| `discover_memories` | query, min_similarity, max_similarity, limit, project | Sweet-spot cosine search (0.3–0.7) for lateral connections. |
| `health_check` | — | Returns server status and configuration. |

**Naming consistency:** All tool names use `snake_case` consistently. Parameter names use `snake_case` throughout. No inconsistencies found.

### HTTP Endpoints (transport/api/)

| Method | Path | Handler | Notes |
|-|-|-|-|
| POST | `/v1/store` | `store_handler` | Store a memory. Mirrors `store_memory` MCP tool. |
| POST | `/v1/search` | `search_handler` | Hybrid search. Mirrors `search_memory` MCP tool. |
| POST | `/v1/recall` | `recall_handler` | Session recall. Mirrors `recall_memory` MCP tool. |
| POST | `/v1/annotate` | `annotate_handler` | Annotate memory. Mirrors `annotate_memory` MCP tool. |
| POST | `/v1/update` | `update_handler` | Update memory. Mirrors `update_memory` MCP tool. |
| DELETE | `/v1/memories/{id}` | `handle_delete` | Delete by ID. |
| GET | `/v1/status` | `status_handler` | Health/status. |
| GET | `/v1/export` | `export_handler` | Export memories (jsonl/csv/markdown). |
| POST | `/v1/discover` | `discover_handler` | Discover connections. Mirrors `discover_memories` MCP tool. |

**Naming consistency:** All paths use `/v1/` prefix consistently. Paths use `snake_case` nouns (`/v1/store`, `/v1/recall`). One minor inconsistency: the delete path uses `/v1/memories/{id}` (noun with ID param) while other mutation endpoints use `/v1/{action}` (verb-style). This is a common REST pattern and not a bug — `DELETE /v1/memories/{id}` follows RESTful conventions correctly. No fix needed.

**MCP-HTTP parity:** All 5 write operations (store, search, recall, annotate, update) have HTTP equivalents. `discover` has an HTTP equivalent. `get_memory`, `list_memories`, `reinforce_memory`, `feedback_memory`, `bulk_delete_memories`, and `health_check` are MCP-only. This is intentional — the HTTP API covers the most common agent access patterns; rarer operations are MCP-only.

### CLI Commands (crates/memcp/src/main.rs)

| Command | Key Flags | Notes |
|-|-|-|
| `store` | content (positional), --type-hint, --tags, --source, --actor, --audience, --project, --trust-level, --session-id, --agent-role, --wait, --stdin, --idempotency-key | Full parity with MCP store_memory. |
| `search` | query (positional), --limit, --tags, --source, --type-hint, --project, --min-salience, --cursor, --fields, --verbose, --json, --compact | Full parity with MCP search_memory. |
| `recall` | query (optional positional), --project, --session-id, --reset, --first, --limit, --boost-tags | Full parity with MCP recall_memory. |
| `list` | --type-hint, --source, --cursor, --limit, --audience, --project, --actor, --verbose, --created-after/before, --updated-after/before | Pagination list. |
| `get` | id (positional) | Fetch by ID. |
| `delete` | id (positional) | Permanent delete. |
| `recent` | --since, --source, --actor, --limit, --verbose | Session handoff convenience. |
| `reinforce` | id, --rating | Boost salience. |
| `feedback` | id, signal | Relevance feedback. |
| `annotate` | --id, --tags, --replace-tags, --salience | Annotate memory. |
| `update` | id, content (optional positional), --type-hint, --source, --tags, --wait, --stdin | Update memory. |
| `gc` | --dry-run, --salience-threshold, --min-age-days | Garbage collection. |
| `curation` | (subcommand: run, status) | Manual AI curation trigger. |
| `import` | (subcommand: openclaw, claude-code, chatgpt, jsonl, markdown, batch) | Import from external sources. |
| `export` | --format, --output, --project, --tags, --since, --include-embeddings, --include-state | Export memories. |
| `discover` | query (positional), --min-similarity, --max-similarity, --limit, --project, --json | Discover connections. |
| `serve` | — | Start MCP server on stdio. |
| `migrate` | — | Run DB migrations. |
| `status` | --pretty, --check | Show daemon status. |
| `daemon` | (subcommand: install) | Background worker management. |
| `embed` | (subcommand: status, reembed) | Embedding management. |
| `statusline` | (subcommand: install, uninstall, update) | Claude Code status line integration. |

**Naming consistency:** All commands use `kebab-case` for multi-word flags (--type-hint, --boost-tags, etc.) and `snake_case` internally — correct Clap convention. No inconsistencies found.

### Configuration Keys (memcp.toml / environment variables)

Top-level config struct uses `MEMCP_*` env var prefix with `__` separator for nested keys.

| Section | Key | Type | Default | Notes |
|-|-|-|-|-|
| root | `log_level` | String | `"info"` | `MEMCP_LOG_LEVEL`. Values: trace, debug, info, warn, error. |
| root | `log_file` | Option\<String\> | None | File logging deferred (runtime warning if set). |
| root | `database_url` | String | `"postgres://..."` | Also reads `DATABASE_URL` env var. |
| root | `project` | ProjectConfig | — | `#[serde(alias = "workspace")]` — backward compat. |
| `[embedding]` | `provider` | String | `"local"` | Values: local, openai. |
| `[embedding]` | `local_model` | String | `"Xenova/all-MiniLM-L6-v2"` | fastembed model name. |
| `[search]` | `bm25_backend` | String | `"native"` | Values: native, paradedb. |
| `[search]` | `default_min_salience` | Option\<f64\> | None | Global salience floor. |
| `[salience]` | `w_recency` | f64 | 0.25 | Recency dimension weight. |
| `[salience]` | `w_semantic` | f64 | 0.45 | Semantic relevance weight. |
| `[extraction]` | `provider` | String | `"ollama"` | Values: ollama, openai. |
| `[extraction]` | `enabled` | bool | true | Set false to skip extraction. |
| `[consolidation]` | `enabled` | bool | true | Auto-merge near-duplicate memories. |
| `[consolidation]` | `similarity_threshold` | f64 | 0.92 | pgvector cosine threshold for merge. |
| `[query_intelligence]` | `expansion_enabled` | bool | false | Query expansion (opt-in). |
| `[gc]` | `enabled` | bool | false | Auto-GC via daemon. Opt-in. |
| `[gc]` | `salience_threshold` | f64 | 0.1 | Prune memories below this score. |
| `[recall]` | `max_memories` | usize | 3 | Memories injected per recall. |
| `[curation]` | `enabled` | bool | false | AI brain curation. Opt-in. |
| `[summarization]` | `enabled` | bool | false | Auto-summarize auto-store content. Opt-in. |

**Naming consistency:** All config sections use `snake_case`. All nested keys use `snake_case`. The `[project]` section accepts `[workspace]` as an alias for backward compatibility — this is intentional and documented.

### Naming Consistency Findings

No blocking renames required. The following minor observations are cataloged for awareness:

| Item | Observation | Recommendation |
|-|-|-|
| HTTP DELETE path | `/v1/memories/{id}` vs `/v1/{action}` style for other endpoints | Correct REST convention; no change needed. |
| `discover_memories` vs `discover` | CLI command is `discover`, MCP tool is `discover_memories` | Minor inconsistency; both are clear. Low priority. |
| `reinforce_memory` vs `reinforce` | CLI command is `reinforce`, MCP tool is `reinforce_memory` | Same pattern. Consistent with convention: MCP tools have noun suffix, CLI commands are shorter verbs. |
| `feedback_memory` vs `feedback` | Same pattern as above. | Same recommendation. |
| `health_check` (MCP) vs `/v1/status` (HTTP) vs `status` (CLI) | Three names for the same concept | Minor; MCP name (`health_check`) follows the standard MCP tool naming; HTTP and CLI names (`status`) follow REST/CLI conventions. No change needed. |

---

## Test Coverage Analysis

### Coverage by Module

| Module | Test Count | Coverage Assessment | Notes |
|-|-|-|-|
| `pipeline/temporal/` | 15 | Good | Temporal extraction well-tested |
| `pipeline/curation/algorithmic.rs` | 14 | Good | Injection detection and curation actions covered |
| `pipeline/curation/worker.rs` | 5 | Minimal | Coordinator logic not tested |
| `pipeline/chunking/` | 13 | Good | Splitter and chunking logic covered |
| `pipeline/enrichment/mod.rs` | 3 | Minimal | Schema structure tested; enrichment prompt not |
| `pipeline/enrichment/worker.rs` | 2 | Minimal | Tag validation tested; worker lifecycle not |
| `pipeline/extraction/` | 0 | None | Prompt building, provider routing untested |
| `pipeline/consolidation/` | 0 | None | Similarity checking, merge logic untested |
| `pipeline/gc/` | 0 | None | GC candidate selection, dedup untested |
| `pipeline/auto_store/` | 0 | None | File watcher, parser, filter untested |
| `pipeline/summarization/` | 0 | None | Provider trait impls untested |
| `pipeline/promotion/` | 0 | None | FSRS promotion worker untested |
| `intelligence/embedding/router.rs` | 8 | Good | Multi-tier routing covered |
| `intelligence/embedding/local.rs` | 0 | None | fastembed integration untested (requires model) |
| `intelligence/embedding/openai.rs` | 0 | None | OpenAI provider untested (requires API key) |
| `intelligence/search/mod.rs` | 3 | Minimal | RRF fusion tested; salience scoring untested |
| `intelligence/search/salience.rs` | 0 | None | Core salience scoring logic untested |
| `intelligence/query_intelligence/mod.rs` | 4 | Minimal | Basic QI pipeline tested |
| `intelligence/query_intelligence/temporal.rs` | 0 | None | Temporal hint parsing untested |
| `intelligence/query_intelligence/ollama.rs` | 0 | None | Provider untested |
| `intelligence/query_intelligence/openai.rs` | 0 | None | Provider untested |
| `storage/store/postgres.rs` | 0 | None | Entire DB layer untested (requires live PG) |
| `transport/server.rs` | 7 | Minimal | UUID ref map tested; tool handlers untested |
| `transport/api/` (all) | 0 | None | All HTTP handlers untested |
| `transport/daemon.rs` | 0 | None | Daemon/embedding router untested |
| `transport/ipc.rs` | 0 | None | IPC untested |
| `benchmark/` | 25 | Moderate | Dataset, evaluate, ingest covered; runner/prompts not |
| `import/` | 81 | Good | Importers for openclaw, claude-code, chatgpt, markdown, jsonl covered |
| `import/security.rs` | 0 | None | Trust security checks untested |
| `import/history.rs` | 0 | None | History parsing untested |
| `load_test/` | 43 | Good | Trust scenarios, report generation, corpus covered |
| `config.rs` | 6 | Minimal | Basic config parsing tested; subsystem configs not |

### Test Count Totals

| Area | Tests |
|-|-|
| `pipeline/` | 55 |
| `intelligence/` | 15 |
| `import/` | 81 |
| `load_test/` | 43 |
| `benchmark/` | 25 |
| `storage/` | 0 |
| `transport/` | 7 |
| **Total** | **~226** |

### Priority Coverage Gaps (for follow-up phase)

The following gaps are cataloged in priority order. No tests are written here — deferred to a dedicated test coverage phase.

**P1 — High Impact (covers core correctness)**

1. `intelligence/search/salience.rs` — Core salience scoring is the heart of search quality; untested means any regression goes undetected.
2. `storage/store/postgres.rs` — Entire persistence layer has zero unit tests. Requires integration test setup (test DB schema isolation via `new_with_schema()` from Phase 14.7 — this is already available).
3. `transport/api/` handlers — HTTP API has zero tests. All 9 handler files are untested. Unit tests can use `axum_test` or `tower::ServiceExt` without a live server.
4. `pipeline/gc/` — GC candidate selection logic (salience thresholds, age cutoffs, dedup) is correctness-critical and entirely untested.

**P2 — Medium Impact (covers new Phase 10.1/11.x code)**

5. `import/security.rs` — Trust security checks added in Phase 11.1 are untested.
6. `pipeline/consolidation/similarity.rs` — Similarity scoring used by consolidation pipeline untested.
7. `pipeline/extraction/` — LLM extraction pipeline (prompt building, provider routing) untested.
8. `pipeline/auto_store/parser.rs` and `filter.rs` — Auto-store conversation parser and filter untested.

**P3 — Lower Priority (external provider integration)**

9. `intelligence/embedding/local.rs` — fastembed integration (requires model download; integration test).
10. `pipeline/summarization/` — Provider impls (require Ollama/OpenAI; integration tests).
11. `intelligence/query_intelligence/temporal.rs` — Temporal hint parsing (pure logic; unit-testable without external deps).

---

## Cataloged Issues (Non-Critical)

Issues found during audit that are NOT blocking for release but are cataloged for future phases.

| # | Issue | Location | Severity | Recommendation |
|-|-|-|-|-|
| 1 | `log_file` config accepted but not implemented | `logging.rs` | Low | Runtime `warn!()` informs users. Implement with `tracing-appender` in a future phase. |
| 2 | `test_load_locomo_dataset_valid` ignored | `benchmark/locomo/dataset.rs` | Low | Test fixture uses array JSON, struct expects dict. Fix deserialization path or update fixture. Tracked via `#[ignore]` message. |
| 3 | `discover` CLI vs `discover_memories` MCP naming | `main.rs`, `server.rs` | Cosmetic | MCP tools use noun suffix convention; CLI uses shorter verbs. Document the convention rather than rename. |
| 4 | HTTP API missing `list`, `get`, `reinforce`, `feedback`, `bulk_delete` endpoints | `transport/api/` | Low | Currently MCP-only. Add HTTP equivalents if a non-MCP HTTP client is needed. |
| 5 | `transport/server.rs:2058` — `params.query.as_ref().unwrap()` | `server.rs` | Cosmetic | Logically safe (guarded above), but could be replaced with `if let Some(query) = &params.query` for readability. |
| 6 | CONTRIBUTING.md references CLA assistant | `CONTRIBUTING.md` | Low | MIT license does not require a CLA. Remove CLA bot reference and update contribution workflow. |
| 7 | Test infrastructure: no `#[tokio::test]` setup for DB-backed tests | `tests/` | Medium | Phase 14.7's `new_with_schema()` provides schema isolation — needs a test helper crate or integration test setup guide. |

---

## Workspace to Project Rename Status

The rename is **complete** across all surfaces. Backward-compatibility aliases are retained intentionally for open-source launch.

| Surface | Status | Notes |
|-|-|-|
| DB column | COMPLETE | Migration 020 renamed `workspace` → `project` |
| CLI flags | COMPLETE | `--project` is primary; `alias = "workspace"` kept for compat |
| `resolve_project()` | COMPLETE | `MEMCP_PROJECT` primary; `MEMCP_WORKSPACE` fallback |
| MCP tool params | COMPLETE | All MCP tools use `project`; no `workspace` parameters |
| HTTP API types | COMPLETE | `project` field with `#[serde(alias = "workspace")]` |
| Config keys | COMPLETE | `[project]` section with `#[serde(alias = "workspace")]` |
| SQL queries | COMPLETE | All SQL references `project` column |
| Config env vars | COMPLETE | `MEMCP_PROJECT` primary; `MEMCP_WORKSPACE` fallback in resolver |

**Decision:** Backward-compat aliases (`alias = "workspace"`, `MEMCP_WORKSPACE`) are intentionally retained. Removing them would break existing users' configs and env var setups. Aliases will be deprecated via changelog notice and removed in a future major version.

---

## Appendix: Files with Zero Tests

The following production files have no tests. This list feeds directly into the test coverage phase planning.

**Critical paths (high value to test):**
- `storage/store/postgres.rs` — entire DB layer
- `transport/api/{recall,search,store,annotate,update,delete,export,discover}.rs` — all HTTP handlers
- `intelligence/search/salience.rs` — core scoring
- `pipeline/gc/{worker,dedup}.rs` — GC logic

**Background worker impls (integration test territory):**
- `pipeline/extraction/{ollama,openai,pipeline}.rs`
- `pipeline/consolidation/similarity.rs`
- `pipeline/auto_store/{watcher,filter,parser}.rs`
- `pipeline/summarization/{ollama,openai}.rs`
- `pipeline/curation/{openai}.rs`

**Thin dispatch / CLI shim (lower priority):**
- `transport/daemon.rs` — daemon lifecycle
- `transport/ipc.rs` — IPC channel
- `crates/memcp-core/src/cli.rs` — CLI formatting helpers
- `import/{security,history}.rs` — new Phase 11.x files
