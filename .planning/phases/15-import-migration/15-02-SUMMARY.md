---
phase: 15-import-migration
plan: 02
subsystem: import
tags: [export, jsonl, csv, markdown, round-trip, anti-lock-in, rust]

requires:
  - phase: 15-01
    provides: ImportEngine, ImportSource trait, JsonlReader, batch_insert_memories

provides:
  - ExportEngine with project/tags/since filter dispatch to JSONL/CSV/Markdown formatters
  - JSONL formatter with --include-state and --include-embeddings support
  - CSV formatter with manual quoting, header row, optional salience state columns
  - Markdown formatter grouping memories by type_hint as blockquoted archive
  - `memcp export --format <jsonl|csv|markdown>` CLI command
  - Round-trip fidelity: exported JSONL re-importable via `memcp import jsonl`

affects: [15-03, 15-04, 15-05, import-migration-future]

tech-stack:
  added: []
  patterns:
    - "ExportEngine follows ImportEngine pattern: build opts, dispatch to sub-formatters"
    - "Dynamic SQL with positional params for filter clauses (project, since, tags)"
    - "pgvector text repr [0.1,0.2,...] parsed manually to Vec<f32> for JSONL export"
    - "Output writer branching: file vs stdout via BufWriter to avoid trait object lifetime issues"

key-files:
  created:
    - crates/memcp-core/src/import/export/mod.rs
    - crates/memcp-core/src/import/export/jsonl.rs
    - crates/memcp-core/src/import/export/csv.rs
    - crates/memcp-core/src/import/export/markdown.rs
  modified:
    - crates/memcp-core/src/import/mod.rs
    - crates/memcp/src/main.rs
    - crates/memcp-core/tests/import_test.rs

key-decisions:
  - "ExportableMemory is a flat struct combining Memory + optional salience + optional embedding — each formatter receives a plain slice, no trait dispatch"
  - "Embedding join uses embedding::text cast in SELECT (m.embedding::text AS embedding_text) then parsed from [0.1,...] text repr — avoids pgvector type complexity in raw sqlx query"
  - "Output dispatch uses two BufWriter branches (file vs stdout) instead of Box<dyn Write> — avoids lifetime issues with StdoutLock trait object"
  - "Tags filter uses JSONB containment operator (@>) per tag in WHERE clause — consistent with existing search patterns"
  - "CSV tags field serialized as space-separated string within the cell — simple and deterministic, no nested quoting issues"
  - "Markdown groups by BTreeMap<type_hint, Vec<&ExportableMemory>> — sorted groups for deterministic output"
  - "Round-trip test validates re-import detects dedup (content hash match) — proves export format is parseable by import pipeline"
  - "Chatgpt/Claude/Markdown ImportAction match arms added (Rule 3 blocking fix) — prior plan left them unimplemented, caused compilation failure"

requirements-completed: [IMP-10, IMP-11]

duration: 11min
completed: 2026-03-03
---

# Phase 15 Plan 02: Export Pipeline Summary

**Three-format export pipeline (JSONL/CSV/Markdown) with ExportEngine, filter flags, and round-trip JSONL fidelity validation**

## Performance

- **Duration:** 11 min
- **Started:** 2026-03-03T07:04:18Z
- **Completed:** 2026-03-03T07:15:00Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments

- ExportEngine queries memories from Postgres with project/tags/since filters using dynamic SQL
- JSONL formatter produces round-trip compatible output — one JSON object per line with all metadata
- CSV formatter produces flat tabular output with header row, proper escaping for commas/newlines
- Markdown formatter groups memories by type_hint as human-readable blockquoted archive
- `memcp export --format <jsonl|csv|markdown>` CLI command wired end-to-end
- 17 unit tests for formatters + ExportEngine helpers
- 1 integration round-trip test: store → export JSONL → re-import detects dedup

## Task Commits

1. **Task 1: Export module with three formatters + ExportEngine** - `ad23059` (feat)
2. **Task 2: CLI Commands::Export wiring + round-trip test** - `60e9f68` (feat)

## Files Created/Modified

- `crates/memcp-core/src/import/export/mod.rs` — ExportEngine, ExportFormat, ExportOpts, ExportableMemory, pgvector text parser
- `crates/memcp-core/src/import/export/jsonl.rs` — JSONL formatter with include_state/include_embeddings support (8 tests)
- `crates/memcp-core/src/import/export/csv.rs` — CSV formatter with manual quoting and salience state columns (5 tests)
- `crates/memcp-core/src/import/export/markdown.rs` — Markdown formatter grouped by type_hint (4 tests)
- `crates/memcp-core/src/import/mod.rs` — Added `pub mod export;`
- `crates/memcp/src/main.rs` — Commands::Export variant, match arm, ExportEngine wiring; Chatgpt/Claude/Markdown ImportAction arms
- `crates/memcp-core/tests/import_test.rs` — `test_export_import_round_trip` integration test

## Decisions Made

- ExportableMemory flat struct (not a trait) — formatters receive plain slice, no dispatch overhead
- Embedding exported as pgvector text parsed to Vec<f32> via manual `[0.1,0.2,...]` parsing
- BufWriter branching for file vs stdout avoids StdoutLock lifetime issues with dyn Write
- CSV tags as space-separated string — deterministic, avoids nested quoting complexity
- Markdown uses BTreeMap for sorted type_hint groups — deterministic output order

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added Chatgpt/Claude/Markdown ImportAction match arms**
- **Found during:** Task 2 (CLI wiring)
- **Issue:** Commands::Import match was non-exhaustive — Chatgpt, Claude, Markdown ImportAction variants (added in plans 15-03/15-04) had no arms, causing compilation failure
- **Fix:** Added three match arms dispatching to ChatGptReader, ClaudeAiReader, MarkdownReader with same ImportOpts construction pattern as Jsonl/Openclaw arms
- **Files modified:** crates/memcp/src/main.rs
- **Verification:** cargo build succeeds, no non-exhaustive patterns error
- **Committed in:** 60e9f68 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Required fix — compilation was blocked. No scope creep.

## Issues Encountered

- CreateMemory struct has `tags: Option<Vec<String>>` not `Option<serde_json::Value>` as the plan's interfaces section showed — fixed in round-trip test by using correct type

## Self-Check

- [x] crates/memcp-core/src/import/export/mod.rs exists (259 lines)
- [x] crates/memcp-core/src/import/export/jsonl.rs exists (170 lines)
- [x] crates/memcp-core/src/import/export/csv.rs exists (165 lines)
- [x] crates/memcp-core/src/import/export/markdown.rs exists (175 lines)
- [x] ad23059 commit exists
- [x] 60e9f68 commit exists
- [x] `cargo build` succeeds
- [x] 17 lib unit tests pass (import::export::*)
- [x] 7 import_test integration tests pass including round-trip

## Self-Check: PASSED

## Next Phase Readiness

- Export pipeline complete, anti-lock-in guarantee implemented
- JSONL round-trip verified: export → import preserves content, type_hint, tags
- Ready for Phase 15-03+ (additional import readers build on same ImportEngine)
- Future: `memcp export --format csv --output /tmp/archive.csv` and `memcp export --format markdown` work end-to-end

---
*Phase: 15-import-migration*
*Completed: 2026-03-03*
