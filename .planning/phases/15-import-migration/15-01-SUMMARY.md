---
phase: 15-import-migration
plan: "01"
subsystem: import
tags: [import, pipeline, jsonl, dedup, noise-filter, batch-insert, checkpoint]
dependency_graph:
  requires:
    - crates/memcp-core/src/storage/store/postgres.rs (PostgresMemoryStore, pool())
    - crates/memcp-core/src/pipeline/auto_store/mod.rs (content_hash FNV-1a pattern)
  provides:
    - crates/memcp-core/src/import/mod.rs (ImportSource trait, ImportEngine, ImportOpts)
    - crates/memcp-core/src/import/noise.rs (NoiseFilter)
    - crates/memcp-core/src/import/dedup.rs (SHA-256 normalized hash, check_existing)
    - crates/memcp-core/src/import/batch.rs (batch_insert_memories)
    - crates/memcp-core/src/import/checkpoint.rs (Checkpoint, ImportReport, import_dir)
    - crates/memcp-core/src/import/jsonl.rs (JsonlReader)
    - `memcp import jsonl <path>` CLI command
  affects:
    - crates/memcp/src/main.rs (Commands::Import added)
    - crates/memcp-core/src/lib.rs (pub mod import registered)
tech_stack:
  added:
    - rusqlite = "0.31" (bundled) — for future OpenClaw SQLite reader
    - zip = "2" — for future ChatGPT/Claude.ai ZIP readers
    - sha2 = "0.10" — SHA-256 normalized content hashing for import dedup
    - chrono added to memcp-bin Cargo.toml (for --since parsing in main.rs)
  patterns:
    - ImportSource trait (async_trait) — pluggable reader interface
    - Three-tier pipeline (noise → dedup → batch INSERT)
    - sqlx::test (#[sqlx::test(migrator = "memcp::MIGRATOR")]) for import integration tests
    - UUID suffix in import_dir() to prevent same-second collision in checkpoint paths
key_files:
  created:
    - crates/memcp-core/src/import/mod.rs
    - crates/memcp-core/src/import/noise.rs
    - crates/memcp-core/src/import/dedup.rs
    - crates/memcp-core/src/import/batch.rs
    - crates/memcp-core/src/import/checkpoint.rs
    - crates/memcp-core/src/import/jsonl.rs
    - crates/memcp-core/tests/import_test.rs
  modified:
    - crates/memcp-core/Cargo.toml
    - crates/memcp-core/src/lib.rs
    - crates/memcp/Cargo.toml
    - crates/memcp/src/main.rs
decisions:
  - "ImportEngine::new() creates NoiseFilter from user opts; source-specific patterns are merged in run() via new_with_source_patterns(). This keeps construction simple while supporting per-source pattern defaults."
  - "check_existing() fetches raw content from DB (no normalized_hash column), computes SHA-256 on the fly, intersects with import batch hashes. Acceptable for one-time import ops; future optimization: add normalized_hash column."
  - "import_dir() uses UUID suffix (first 8 chars) + timestamp to guarantee uniqueness per run. Fixed bug where same-second runs would share checkpoint directory causing second import to resume the first."
  - "ImportEngine stores _noise_filter with underscore prefix (prefixed with _ to suppress dead_code warning) — field is intentionally kept for future use by OpenClaw/ClaudeCode readers that need source-level noise config."
  - "batch_insert_memories merges chunk.tags + opts.tags + auto-source-tags ('imported', 'imported:<source>'), deduplicates with sort/dedup, stores as JSONB."
  - "Batch embedding insert uses pgvector literal format '[x,y,z]::vector' for pre-computed embeddings; falls back to embedding_status='pending' on insert failure (fail-open)."
  - "todo!() used for Discover/Review/Rescue ImportAction variants — Plan 05 implements these."
metrics:
  duration: "13 minutes"
  completed: "2026-03-03"
  tasks: 2
  files: 11
---

# Phase 15 Plan 01: Import Infrastructure + JSONL Reader Summary

Core import infrastructure built: ImportSource trait, three-tier pipeline (noise → SHA-256 dedup → batch INSERT), JSONL reader for round-trip validation, CLI `memcp import jsonl` command, checkpoint/resume, progress bar.

## What Was Built

### Import Module (crates/memcp-core/src/import/)

**mod.rs** — Core types and ImportEngine pipeline driver:
- `ImportSource` trait (async_trait): `source_name()`, `source_kind()`, `noise_patterns()`, `discover()`, `read_chunks()`
- `ImportChunk` struct: content, type_hint, source, tags, created_at, actor, embedding, embedding_model, workspace
- `ImportOpts` struct: project, tags, skip_embeddings, batch_size (default 100), since, dry_run, curate, skip_patterns
- `ImportResult` / `ImportError` / `DiscoveredSource` types
- `ImportSourceKind` enum: OpenClaw, ClaudeCode, ChatGpt, ClaudeAi, Markdown, Jsonl
- `ImportEngine::run()`: noise filter → since filter → dedup → batch insert → checkpoint → report

**noise.rs** — Rule-based noise filter:
- `NoiseFilter::is_noise()`: min_chars=50 + case-insensitive contains matching
- OpenClaw hardcoded patterns: HEARTBEAT_OK, Token Monitor Report, Switchboard - Cross-Subagent, FailoverError: LLM request timed out, Exec failed, Exec completed, compinit: initialization aborted

**dedup.rs** — SHA-256 normalized dedup:
- `normalize_content()`: strips markdown (#, *, _, >, ```, - list markers), lowercases, collapses whitespace
- `normalized_hash()`: SHA-256 of normalized content → hex string
- `check_existing()`: queries memories from last 30 days, computes hashes on fly, returns intersection

**batch.rs** — Direct Postgres batch INSERT:
- `batch_insert_memories()`: single transaction per batch, UUID id, FNV-1a content_hash, merged tags, embedding_status based on chunk data
- Embedding insert into memory_embeddings when chunk.embedding is Some
- ON CONFLICT DO NOTHING safety net

**checkpoint.rs** — Resume + reporting:
- `Checkpoint` struct: save/load as JSON, stores progress per batch
- `ImportReport` struct: write_report() generates report.json
- `import_dir()`: `~/.memcp/imports/<source>-<timestamp>-<uuid8>/` — unique per run

**jsonl.rs** — JSONL reader implementing ImportSource:
- Line-by-line JSON parsing, skips blank/comment lines, collects parse errors (fail-open)
- Respects opts.since filter, preserves all metadata from JSONL format

### CLI (crates/memcp/src/main.rs)

```
memcp import jsonl <path> [--dry-run] [--project P] [--tags a,b] [--skip-embeddings]
                          [--batch-size 100] [--since ISO8601] [--skip-pattern a,b]
memcp import discover     (placeholder — Plan 05)
memcp import review --last (placeholder — Plan 05)
memcp import rescue [id] [--all] (placeholder — Plan 05)
```

### Integration Tests (tests/import_test.rs)

6 tests using `#[sqlx::test(migrator = "memcp::MIGRATOR")]`:
- `test_import_jsonl_end_to_end`: full pipeline, 5 memories imported, auto-tags applied
- `test_import_dry_run_does_not_write`: dry-run reports would-import but writes 0 to DB
- `test_import_dedup_on_reimport`: second import of same file = 0 new, 5 dedup-skipped
- `test_import_noise_filter_drops_short_content`: 2 short memories filtered, 1 stored
- `test_import_with_project_and_extra_tags`: project scope, CLI tags merged into all memories
- `test_dedup_check_existing_finds_stored_content`: verify check_existing() pool query

19 unit tests across noise.rs, dedup.rs, batch.rs, checkpoint.rs, jsonl.rs.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed same-second checkpoint collision causing dedup test failure**
- **Found during:** Task 2 (integration test debugging)
- **Issue:** `import_dir()` used second-precision timestamp only. Two `engine.run()` calls within the same second (as in tests) shared the same import dir. Second run found the first run's checkpoint and resumed from batch 1, carrying over `result_so_far.imported=5`, so dedup appeared broken.
- **Fix:** Added UUID 8-char suffix to `import_dir()` → `<source>-<timestamp>-<uuid8>/`
- **Files modified:** `crates/memcp-core/src/import/checkpoint.rs`
- **Commit:** a8ef257

## Self-Check: PASSED

All created files exist on disk. All commits verified in git log.

| Item | Status |
|-|-|
| crates/memcp-core/src/import/mod.rs | FOUND |
| crates/memcp-core/src/import/noise.rs | FOUND |
| crates/memcp-core/src/import/dedup.rs | FOUND |
| crates/memcp-core/src/import/batch.rs | FOUND |
| crates/memcp-core/src/import/checkpoint.rs | FOUND |
| crates/memcp-core/src/import/jsonl.rs | FOUND |
| crates/memcp-core/tests/import_test.rs | FOUND |
| Commit d123ee7 (Task 1) | FOUND |
| Commit a8ef257 (Task 2) | FOUND |
