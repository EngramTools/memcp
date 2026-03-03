---
phase: 15-import-migration
plan: 03
subsystem: import
tags: [rusqlite, openclaw, claude-code, embedding-reuse, sqlite, spawn-blocking, section-chunking]

requires:
  - phase: 15-01
    provides: ImportSource trait, ImportEngine pipeline, ImportChunk/ImportOpts structs, noise filter

provides:
  - OpenClawReader with SQLite reading, embedding reuse, and agent filtering
  - ClaudeCodeReader with MEMORY.md section chunking and history.jsonl parsing
  - discover_all_sources() with non-local export instructions
  - memcp import openclaw and memcp import claude-code CLI subcommands wired end-to-end

affects:
  - 15-04 (chatgpt/claude-ai/markdown readers follow same spawn_blocking + ImportSource pattern)
  - 15-05 (review/rescue use openclaw/claude-code import results)

tech-stack:
  added: []
  patterns:
    - "spawn_blocking wraps all sync SQLite/file reads to avoid blocking async executor"
    - "Section-based chunking for MEMORY.md: split on # and ## headers, long sections use chunk_content()"
    - "Embedding reuse: compare stored model name + vector dimension before reusing existing vectors"
    - "Agent name extraction from path: sessions/{agent}/date → agent, memory/{date} → file stem fallback"
    - "Integer millisecond timestamps: OpenClaw stores updated_at as INTEGER ms, not ISO 8601 string"

key-files:
  created:
    - crates/memcp-core/src/import/openclaw.rs
    - crates/memcp-core/src/import/claude_code.rs
  modified:
    - crates/memcp-core/src/import/mod.rs (registered pub mod openclaw and pub mod claude_code)
    - crates/memcp/src/main.rs (Openclaw and ClaudeCode ImportAction variants + match arms)

key-decisions:
  - "OpenClawReader.updated_at_ms: i64 INTEGER milliseconds — OpenClaw stores ms timestamps not ISO 8601 strings; DateTime::from_timestamp(ms/1000, ns) converts correctly"
  - "ClaudeCodeReader section splitting: manual split on # and ## header lines instead of sentence chunking — preserves semantic structure of user-curated MEMORY.md files"
  - "Embedding reuse gated on model name AND dimension — both must match or chunk gets embedding_status=pending"
  - "Agent name from sessions path: sessions/vita/2026-01-15 → vita; memory/ sources fall back to SQLite filename stem"
  - "discover_all_sources includes static non-local entries (ChatGPT, Claude.ai) with export instructions — unified discovery output regardless of source type"

patterns-established:
  - "OpenClaw/ClaudeCode readers follow JsonlReader pattern: spawn_blocking for sync I/O, fail-open on parse errors, since filter applied in read_chunks"

requirements-completed: [IMP-06, IMP-07]

duration: 12min
completed: 2026-03-03
---

# Phase 15 Plan 03: OpenClaw and Claude Code Readers Summary

**OpenClawReader imports 3,830 SQLite chunks with embedding reuse and agent filtering; ClaudeCodeReader imports MEMORY.md files chunked by section headers with history.jsonl opt-in**

## Performance

- **Duration:** 12 min
- **Started:** 2026-03-03T07:04:45Z
- **Completed:** 2026-03-03T07:17:14Z
- **Tasks:** 2 tasks + 1 auto-fix deviation
- **Files modified:** 4

## Accomplishments
- OpenClawReader reads all 3,830 chunks from ~/.openclaw/memory/main.sqlite; 1,392 pass noise filtering (2,438 noise-filtered heartbeats/operational signals)
- ClaudeCodeReader imports MEMORY.md files section-by-section from 6 discovered per-project locations
- `memcp import --discover` shows real OpenClaw database (3,830 chunks) and 6 Claude Code MEMORY.md sources automatically
- Embedding reuse zero-cost path: existing OpenClaw vectors reused when model+dimension match configured memcp model

## Task Commits

1. **Task 1: OpenClawReader** - `544713a` (feat)
2. **Task 2: ClaudeCodeReader + CLI wiring** - `3623354` (feat)
3. **Deviation: Fix updated_at INTEGER parsing** - `e511b5e` (fix)

## Files Created/Modified
- `crates/memcp-core/src/import/openclaw.rs` - OpenClawReader with SQLite parsing, embedding reuse, agent filtering, discover()
- `crates/memcp-core/src/import/claude_code.rs` - ClaudeCodeReader with MEMORY.md section chunking, history.jsonl opt-in, discover()
- `crates/memcp-core/src/import/mod.rs` - pub mod openclaw and pub mod claude_code registered
- `crates/memcp/src/main.rs` - Openclaw and ClaudeCode ImportAction variants with all common flags + match arms

## Decisions Made
- OpenClaw `updated_at` is INTEGER milliseconds since epoch, not ISO 8601 string — parsed via `DateTime::from_timestamp(ms/1000, ns)` after discovering schema empirically
- MEMORY.md section chunking is header-based (not sentence-based) to preserve semantic structure of user-curated knowledge
- Embedding reuse requires exact match on both model name AND vector dimension — conservative to avoid cross-model cosine incompatibility
- Agent name extracted from sessions/{agent}/date path pattern; memory/ sources fall back to SQLite filename stem (e.g., "main" from main.sqlite)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed OpenClaw updated_at INTEGER vs STRING mismatch**
- **Found during:** Task 2 verification (memcp import openclaw --dry-run showed 0 items)
- **Issue:** CONTEXT.md described updated_at as a string field, but the real OpenClaw SQLite schema stores it as INTEGER milliseconds since Unix epoch. Row.get::<Option<String>>(6) silently returned None for every row, producing zero chunks.
- **Fix:** Changed ChunkRow.updated_at to updated_at_ms: Option<i64>, used row.get(6).ok() to handle rusqlite type coercion, parsed with DateTime::from_timestamp(ms/1000, ns)
- **Files modified:** crates/memcp-core/src/import/openclaw.rs
- **Verification:** memcp import openclaw --dry-run now shows 1392 items (2438 noise-filtered from 3830 total)
- **Committed in:** e511b5e

---

**Total deviations:** 1 auto-fixed (Rule 1 - bug fix)
**Impact on plan:** Critical fix — without it, zero chunks would import. Schema discovery from real data was required.

## Issues Encountered
- Background processes (other parallel agents) modified mod.rs and main.rs during execution, adding chatgpt/claude-ai/markdown readers and CLI wiring (15-04 plan work). This was handled correctly: the claude_code.rs reader was the missing piece that made the already-committed CLI wiring functional.

## Next Phase Readiness
- OpenClaw and Claude Code readers fully functional with dry-run verified against real data
- Discovery shows all local sources automatically — the "zero-config onboarding" demo moment is working
- Plan 15-04 (ChatGPT, Claude.ai, Markdown) readers were already committed by parallel agent
- Plan 15-05 (review/rescue/curate) can build on stable ImportEngine foundation

## Self-Check: PASSED

- FOUND: crates/memcp-core/src/import/openclaw.rs
- FOUND: crates/memcp-core/src/import/claude_code.rs
- FOUND: .planning/phases/15-import-migration/15-03-SUMMARY.md
- FOUND commit: 544713a (feat: OpenClawReader)
- FOUND commit: 3623354 (feat: ClaudeCodeReader)
- FOUND commit: e511b5e (fix: updated_at INTEGER parsing)

---
*Phase: 15-import-migration*
*Completed: 2026-03-03*
