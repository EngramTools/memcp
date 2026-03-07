---
phase: 14-memory-boosting
plan: "04"
subsystem: pipeline/enrichment
tags: [enrichment, daemon-worker, neighbor-tags, llm, provenance]
dependency_graph:
  requires: [14-02]
  provides: [enrichment-worker, EnrichmentProvider, get_unenriched_memories]
  affects: [transport/daemon, pipeline/enrichment, storage/store/postgres]
tech_stack:
  added: [OllamaEnrichmentProvider, EnrichmentConfig, run_enrichment, get_unenriched_memories]
  patterns: [daemon-sweep-worker, fail-open-per-memory, provenance-tag, tag-sanitization]
key_files:
  created:
    - crates/memcp-core/src/pipeline/enrichment/mod.rs
    - crates/memcp-core/src/pipeline/enrichment/worker.rs
  modified:
    - crates/memcp-core/src/config.rs (EnrichmentConfig struct + test cases — pre-existing)
    - crates/memcp-core/src/pipeline/mod.rs (register enrichment module)
    - crates/memcp-core/src/lib.rs (re-export pipeline::enrichment)
    - crates/memcp-core/src/storage/store/postgres.rs (add get_unenriched_memories)
    - crates/memcp-core/src/transport/daemon.rs (wire enrichment worker at step 8.65)
decisions:
  - "Reuse QI config (Ollama base_url + reranking model) for enrichment provider — avoids new config surface"
  - "No-neighbor memories get 'enriched' marker to prevent re-scanning on next sweep"
  - "LlmUnavailable skips memory without marking enriched (retry when LLM returns); ProviderError also skips"
  - "Tag validation: lowercase + alphanumeric + hyphen/underscore, max 50 chars — reject all others"
  - "std::mem::forget(shutdown_tx) keeps enrichment running for daemon lifetime; process exit handles shutdown"
metrics:
  duration_minutes: 20
  tasks_completed: 2
  files_created: 2
  files_modified: 5
  tests_added: 7
  completed_date: "2026-03-07T06:16:00Z"
---

# Phase 14 Plan 04: Retroactive Neighbor Enrichment Summary

**One-liner:** Background daemon sweep that enriches existing memories with neighbor-derived tags using Ollama structured output, provenance-tracked with 'enriched' marker.

## What Was Built

### EnrichmentProvider trait + Ollama implementation (`pipeline/enrichment/mod.rs`)

- `EnrichmentProvider` async trait: `suggest_tags(memory_content, neighbor_contents) -> EnrichmentResult`
- `OllamaEnrichmentProvider`: calls `/api/chat` with `format: enrichment_schema()` for structured JSON output
- `build_enrichment_prompt()`: prompt instructs LLM to find connecting themes between memory and neighbors, output 1–5 concise tags not already obvious from content
- `enrichment_schema()`: JSON schema `{"tags_to_add": ["string"], maxItems: 5}`
- `create_enrichment_provider(qi_config)`: factory reusing QI Ollama config — returns `None` if unconfigured

### Enrichment worker sweep (`pipeline/enrichment/worker.rs`)

- `run_enrichment()`: daemon loop with `watch::Receiver<bool>` shutdown, `tokio::time::interval` for periodic sweeps
- `run_enrichment_sweep()` inner function:
  1. `get_unenriched_memories(batch_limit)` — memories without 'enriched' tag
  2. `get_memory_embedding()` — skip if no embedding yet
  3. `find_similar_memories()` — nearest neighbors via pgvector
  4. No-neighbor memories: marked 'enriched' immediately (prevent re-scan)
  5. `suggest_tags()` LLM call — fail-open per memory
  6. Tag sanitization: `to_lowercase()`, regex `^[a-zA-Z0-9_-]+$`, max 50 chars
  7. Apply tags + 'enriched' provenance marker

### PostgreSQL storage (`storage/store/postgres.rs`)

- `get_unenriched_memories(limit)`: queries `WHERE deleted_at IS NULL AND embedding_status = 'complete' AND NOT (tags @> '["enriched"]'::jsonb) ORDER BY created_at DESC`

### Daemon wiring (`transport/daemon.rs`)

- Spawned at step 8.65 (after curation worker)
- Config-gated: `if config.enrichment.enabled`
- Falls back gracefully if QI provider not configured (logs and continues)
- Logs sweep_interval_secs, batch_limit, neighbor_depth on startup

## Tests

| Test | Location | Validates |
|-|-|-|
| test_enrichment_config_defaults | config::tests | enabled=false, batch_limit=50, sweep=3600, depth=5, threshold=0.7 |
| test_config_has_enrichment_field | config::tests | Config struct has enrichment field |
| test_build_enrichment_prompt_contains_memory | pipeline::enrichment | Prompt includes memory content and neighbors |
| test_build_enrichment_prompt_no_neighbors | pipeline::enrichment | No-neighbor fallback text |
| test_enrichment_schema_structure | pipeline::enrichment | Schema type, maxItems, required |
| test_valid_tag_filter | pipeline::enrichment::worker | Regex accepts a-z0-9_- only |
| test_tag_length_limit | pipeline::enrichment::worker | 50-char max boundary |

## Deviations from Plan

### Auto-fixed Issues

None — plan executed exactly as written with one intentional implementation choice:

**[Rule 2 - Missing functionality] No-neighbor memories marked enriched to prevent infinite re-scanning**
- **Found during:** Task 2 implementation
- **Issue:** Plan said `if neighbors.is_empty() { continue }` — but this would re-scan no-neighbor memories every sweep
- **Fix:** Mark no-neighbor memories with 'enriched' tag immediately (they can't benefit from enrichment anyway)
- **Files modified:** worker.rs

## Self-Check: PASSED

- `pipeline/enrichment/mod.rs` — FOUND
- `pipeline/enrichment/worker.rs` — FOUND
- Commit 557edca (trait + scaffolding) — FOUND
- Commit de94c15 (worker + daemon) — FOUND
- `cargo build` — clean (0 errors, only pre-existing warnings)
- 7 enrichment tests pass (5 unit + 2 config)
