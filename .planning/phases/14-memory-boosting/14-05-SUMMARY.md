---
phase: 14-memory-boosting
plan: 05
subsystem: storage, transport, intelligence
tags: [discovery, cosine-similarity, sweet-spot, mcp-tool, cli, http-api, llm-explanations]
dependency_graph:
  requires: [14-01, 14-03]
  provides: [discover_associations, discover_memories_tool, POST_v1_discover, memcp_discover_cli]
  affects: [transport/server.rs, transport/api, cli.rs, storage/store/postgres.rs, query_intelligence]
tech_stack:
  added: []
  patterns: [HNSW-friendly-post-filter, fail-open-LLM, cosine-sweet-spot-discovery]
key_files:
  created:
    - crates/memcp-core/src/transport/api/discover.rs
    - crates/memcp-core/tests/discover_test.rs
  modified:
    - crates/memcp-core/src/storage/store/postgres.rs
    - crates/memcp-core/src/transport/server.rs
    - crates/memcp-core/src/transport/api/mod.rs
    - crates/memcp-core/src/intelligence/query_intelligence/mod.rs
    - crates/memcp-core/src/intelligence/query_intelligence/ollama.rs
    - crates/memcp-core/src/cli.rs
    - crates/memcp/src/main.rs
    - crates/memcp-core/tests/common/builders.rs
decisions:
  - "Used HNSW-friendly ORDER BY + LIMIT + post-filter approach (3x over-fetch) instead of range filter in SQL — pgvector HNSW index doesn't support WHERE clause filters well"
  - "Added explain_connections() as a default trait method (returns empty vec) so existing providers don't require changes — Ollama implements the full version"
  - "Discover uses qi_expansion_provider (not qi_reranking_provider) for explanations since expansion provider is more likely to be configured for creative tasks"
  - "CLI cmd_discover requires daemon for embedding (same pattern as cmd_recall) — no degraded BM25 fallback since discovery is inherently vector-only"
  - "project filter uses (project = $N OR project IS NULL) — global memories visible in all project scopes, consistent with search behavior"
metrics:
  duration: 9 minutes
  completed: 2026-03-07T06:35:00Z
  tasks_completed: 2
  files_modified: 8
  files_created: 2
---

# Phase 14 Plan 05: Creative Association Discovery Summary

Implemented cosine sweet-spot discovery — a new query mode that finds unexpectedly related memories in the 0.3-0.7 cosine similarity range.

## What Was Built

**TDD: discover_associations() in PostgresMemoryStore** — finds memories in a configurable cosine similarity range using HNSW-friendly ORDER BY + LIMIT + post-filter. Fetches 3x limit to survive post-filter attrition. Returns `Vec<(Memory, f64)>` sorted by similarity descending. Respects project filter (project-scoped + global memories).

**discover_memories MCP tool** — calls discover_associations(), optionally generates LLM connection explanations via qi_expansion_provider (fail-open), injects UUID ref mapping on all results. Response: `{discoveries, query, similarity_range, count}`.

**POST /v1/discover HTTP endpoint** — same discovery logic via axum handler. Embeds query via in-process embedding provider. Returns same JSON shape as MCP tool.

**memcp discover CLI subcommand** — embeds query via daemon IPC (same as recall), prints human-readable results or JSON with `--json`. Requires daemon for embedding.

**explain_connections() trait method** — added as a default (empty) method to QueryIntelligenceProvider, implemented with Ollama provider. Builds a single LLM call to explain why each discovered memory represents an interesting connection to the query topic.

## Commits

| Hash | Description |
|-|-|
| a9d8dea | feat(14-05): implement discover_associations() in PostgresMemoryStore |
| de80afe | feat(14-05): add discover_memories MCP tool + CLI + HTTP API |

## Tests

4 integration tests in `discover_test.rs`:
- `test_discover_empty_store` — empty store returns empty vec
- `test_discover_sweet_spot` — 4 memories at different similarities, only 2 in [0.3, 0.7] returned
- `test_discover_excludes_near_far` — memories only outside sweet spot returns empty
- `test_discover_respects_project_filter` — project-scoped memories isolated correctly

## Deviations from Plan

**Added `explain_connections()` to QueryIntelligenceProvider trait** — Plan showed standalone helper function; actual implementation added as a default trait method. Avoids duplicating HTTP client code from Ollama/OpenAI providers. Both approaches are functionally equivalent.

**Used `qi_expansion_provider` for explanations** (plan said `qi_provider` which doesn't exist as a single field). Server.rs has separate `qi_expansion_provider` and `qi_reranking_provider` fields — used expansion provider since it's more commonly configured.

**Added `project()` setter to MemoryBuilder** — test helper enhancement needed for the project filter test. Backwards compatible.

## Self-Check: PASSED

All artifacts verified:
- discover.rs: FOUND
- discover_test.rs: FOUND
- discover_associations() in postgres.rs: FOUND
- discover_memories tool in server.rs: FOUND
- commit a9d8dea: FOUND
- commit de80afe: FOUND
