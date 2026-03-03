---
phase: 15-import-migration
plan: 05
subsystem: import
tags: [import, review, rescue, remote, config, integration-tests]
dependency_graph:
  requires: [15-01, 15-03, 15-04]
  provides: [import-review, import-rescue, remote-import, import-config]
  affects: [import-pipeline, cli-commands, config]
tech_stack:
  added: []
  patterns: [FilteredItem persistence via JSONL, remote dispatch via dispatch_remote, ImportConfig serde(default)]
key_files:
  created: []
  modified:
    - crates/memcp-core/src/import/checkpoint.rs
    - crates/memcp-core/src/import/mod.rs
    - crates/memcp-core/src/import/noise.rs
    - crates/memcp-core/src/config.rs
    - crates/memcp/src/main.rs
    - crates/memcp-core/tests/import_test.rs
decisions:
  - "FilteredItem appended to filtered.jsonl per-item (not batched) — simpler, atomicity not needed for audit log"
  - "Remote mode skips store-level dedup — remote daemon handles its own dedup on ingest"
  - "rescue --all uses batch_insert_memories locally, dispatch_remote per-item for remote — no new batch endpoint needed"
  - "ImportConfig.batch_size sets default for opts construction; CLI --batch-size always wins (clap default_value overrides config)"
  - "find_latest_import_dir sorts by directory name descending — timestamp in dir name ensures lexicographic = chronological order"
  - "LLM-skipped items (CurationAction::Skip) also persisted to filtered.jsonl with reason=llm:skip"
  - "NoiseFilter::patterns() accessor added to expose patterns for reason string construction in mod.rs"
metrics:
  duration_minutes: 19
  completed_date: "2026-03-03"
  tasks_completed: 2
  files_created: 0
  files_modified: 6
  tests_added: 4
  tests_passing: 11
---

# Phase 15 Plan 05: Review/Rescue + Remote Import + ImportConfig

One-liner: Complete import feedback loop with filtered item persistence to JSONL, review/rescue CLI commands, --remote HTTP dispatch for hosted imports, and user-configurable noise patterns via [import] config section.

## What Was Built

### Task 1: Review/rescue commands + filtered item persistence

**checkpoint.rs** — Extended with `FilteredItem` struct (id, content, reason, source, tags, type_hint, rescued fields). Three new functions:
- `FilteredItem::append(dir, item)` — appends to filtered.jsonl (creates file if missing)
- `load_filtered(dir)` — reads all lines, skips malformed with warning, returns `Vec<FilteredItem>`
- `save_filtered(dir, items)` — rewrites entire filtered.jsonl (used to mark rescued=true)
- `find_latest_import_dir()` — scans `~/.memcp/imports/`, sorts by dir name descending, returns most recent

**mod.rs** — Noise-filtered chunks now persist as `FilteredItem` records. Reason prefix: `noise:too-short` for length filter, `noise:<matched-pattern>` for pattern match. LLM-curated `CurationAction::Skip` items also persisted with `reason="llm:skip"`.

**noise.rs** — Added `patterns()` accessor (`pub fn patterns(&self) -> &[String]`) to expose patterns for reason string construction.

**main.rs** — Wired `ImportAction::Review { last }`:
1. Resolves latest import dir via `find_latest_import_dir()`
2. Loads filtered.jsonl, skips rescued items
3. Prints breakdown by reason prefix (noise/llm/dedup) to stderr
4. Prints each item as `[short-id] reason | preview` to stderr
5. Outputs full JSON array to stdout (programmatic use)

`ImportAction::Rescue { id, all }`:
1. Loads filtered.jsonl from latest import dir
2. If `--all`: converts all unrescued FilteredItems to ImportChunks
3. If `<id>` given: matches full UUID or 8-char prefix
4. Batch inserts via `batch_insert_memories` (local) or per-item `dispatch_remote` (remote)
5. Marks rescued items with `rescued=true` in filtered.jsonl via `save_filtered`

### Task 2: --remote flag + ImportConfig + integration tests

**ImportOpts** — Added `remote_url: Option<String>` field. `Default` impl sets to `None`.

**ImportEngine::run()** — Step 4 skips store-level dedup in remote mode (remote daemon handles its own dedup). Step 6 conditional: if `remote_url` is Some, POSTs each chunk to `/v1/store` via `dispatch_remote`; otherwise uses `batch_insert_memories`. Tags built identically to local path.

**config.rs** — Added `ImportConfig` struct with `noise_patterns: Vec<String>`, `batch_size: usize`, `default_project: Option<String>`. All fields `#[serde(default)]`. Added `import: ImportConfig` field to `Config` struct and `Default` impl.

**main.rs** — All 7 import match arms updated:
- `skip_patterns` extended with `config.import.noise_patterns`
- `project` falls back to `config.import.default_project.clone()`
- `remote_url: cli.remote.clone()` set on all opts
- Rescue arm uses inline `use memcp::import::batch::batch_insert_memories` for local path

**Integration tests (4 new):**
- `test_config_noise_patterns_applied` — verifies custom skip_patterns filter matching content
- `test_filtered_item_roundtrip` — `FilteredItem::append` + `load_filtered` round-trip
- `test_rescue_marks_item_as_rescued` — `save_filtered` correctly marks items as rescued=true
- `test_find_latest_import_dir_empty` — function doesn't panic, returns valid path or None

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Stash pop conflict reverted Task 2 changes**
- Found during: git stash test verification (checking api_test pre-existing failures)
- Issue: `git stash pop` failed due to Cargo.lock conflict, dropping all in-progress Task 2 edits
- Fix: Re-applied all Task 2 changes manually after dropping the failed stash
- Files modified: mod.rs, config.rs, main.rs, import_test.rs
- Commit: 95a10d5

**2. [Rule 2 - Missing] Added NoiseFilter::patterns() accessor**
- Found during: Task 1 implementation (needed to extract matched pattern for FilteredItem.reason)
- Issue: NoiseFilter.patterns field was private, no way to access for reason string construction
- Fix: Added `pub fn patterns(&self) -> &[String]` method
- Files modified: crates/memcp-core/src/import/noise.rs
- Commit: 84584f8

**3. [Out of Scope] Pre-existing api_test failures**
- Found during: full test suite run
- Issue: api_test.rs (14 tests) fail due to axum router path segment format issue (`:{name}` vs `{name}` format)
- Action: Verified pre-existing (same failures before Task 1 commit), logged as deferred
- Not fixed: outside scope of Plan 05

## Self-Check: PASSED

| Check | Result |
|-|-|
| FilteredItem struct in checkpoint.rs | FOUND |
| load_filtered, save_filtered, find_latest_import_dir | FOUND |
| ImportConfig in config.rs | FOUND |
| remote_url field on ImportOpts | FOUND |
| Review/rescue wired in main.rs | FOUND |
| Commit 84584f8 (Task 1) | FOUND |
| Commit 95a10d5 (Task 2) | FOUND |
| cargo build | PASS |
| import integration tests (11 passing) | PASS |
| memcp import review --help | PASS |
| memcp import rescue --help | PASS |
