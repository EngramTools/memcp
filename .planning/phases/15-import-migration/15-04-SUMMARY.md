---
phase: 15-import-migration
plan: 04
subsystem: import
tags: [import, chatgpt, claude-ai, markdown, curation, llm-triage, cli]
dependency_graph:
  requires: [15-01]
  provides: [chatgpt-reader, claude-ai-reader, markdown-reader, import-curation]
  affects: [import-pipeline, cli-commands]
tech_stack:
  added: [zip crate (already present), curate module]
  patterns: [ImportSource trait, chunk_content helper, SummarizationProvider reuse]
key_files:
  created:
    - crates/memcp-core/src/import/chatgpt.rs
    - crates/memcp-core/src/import/claude_ai.rs
    - crates/memcp-core/src/import/markdown.rs
    - crates/memcp-core/src/import/curate.rs
  modified:
    - crates/memcp-core/src/import/mod.rs
    - crates/memcp/src/main.rs
decisions:
  - "chunk_content() placed in chatgpt.rs as pub fn, shared via pub use by claude_ai and markdown — avoids duplicate util module"
  - "ClaudeAiReader handles both per-file and bulk conversations.json ZIP formats — iterates all .json entries, parses each by heuristic"
  - "flatten_conversation DFS walks mapping parent-child chain; fallback to insertion order when no clear root found"
  - "curate module reuses create_summarization_provider (SummarizationConfig already in Config) — no new config keys"
  - "ImportEngine::with_curator() builder pattern — curator is None when --curate not set, avoids changing new() signature"
  - "Tier 2 curation at step 3.5 — after noise filter, before dedup — LLM-curated content still goes through dedup"
  - "Conversation sources (ChatGPT/Claude.ai) use summarize_conversation(); all others use classify_batch() — source_kind() discriminates"
  - "parse_since() helper extracted at end of main.rs — eliminates repeated --since boilerplate across 5 match arms"
  - "sanitize_tag trailing dashes trimmed by trim_matches('-') — test was wrong, fixed to match actual correct behavior"
metrics:
  duration_minutes: 9
  completed_date: "2026-03-03"
  tasks_completed: 2
  files_created: 4
  files_modified: 2
  tests_added: 20
  tests_passing: 81
---

# Phase 15 Plan 04: ChatGPT/Claude.ai/Markdown readers + Tier 2 LLM triage

One-liner: ChatGPT ZIP reader with mapping-chain flattening, Claude.ai ZIP reader handling both bulk/per-file formats, Markdown section-based chunker, and Tier 2 LLM curation reusing existing SummarizationProvider.

## What Was Built

### Task 1: Three source readers

**chatgpt.rs** — `ChatGptReader` reads ZIP exports containing `conversations.json`. Finds the file by name (not index position), buffers content to memory, flattens the mapping graph via DFS from root node. Chunked to 2048 chars or preserved as single block for `--curate`. Tags include `conversation:<sanitized-title>`.

**claude_ai.rs** — `ClaudeAiReader` reads ZIP exports with either per-file (one JSON per conversation) or bulk (`conversations.json`) format. Iterates all `.json` entries, detects format by filename. Flattens `chat_messages` array with `human`/`assistant` role labels.

**markdown.rs** — `MarkdownReader` handles single files and directories (recursive glob). Splits content at `# ` and `## ` header boundaries via `split_by_headers()`. Long sections further chunked by `chunk_content()`. `type_hint=fact` for all markdown content.

All three registered in `import/mod.rs`.

### Task 2: Tier 2 LLM curation + CLI wiring

**curate.rs** — `ImportCurator` wrapping `Arc<dyn SummarizationProvider>`. Two modes:
- `classify_batch()`: sends up to 50 chunks per LLM call with classification prompt (`keep|skip|merge <type_hint> [topic]`). Parses response lines by chunk index. Fail-open on any parse error.
- `summarize_conversation()`: produces a single distilled memory from a full conversation.

`ImportCurator::new(config)` returns `Option<Self>` — None when no provider configured, with warning.

**ImportEngine** updated with optional `curator` field and `with_curator()` builder. Step 3.5 in `run()` applies curation after noise filter: conversation sources (ChatGPT/ClaudeAi by `source_kind()`) use `summarize_conversation()`; all other sources use `classify_batch()` to keep/skip/merge.

**main.rs** — Added `Chatgpt`, `Claude`, `Markdown` `ImportAction` variants with `--curate` flag and full common flags. Wired match arms for all three plus the Plan 03 `Openclaw` and `ClaudeCode` variants that were missing. `parse_since()` helper at file end.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed wrong sanitize_tag test assertion**
- Found during: Task 2 (cargo test)
- Issue: Test expected `"hello-world-"` but `trim_matches('-')` correctly trims the trailing dash from `!` → `-`
- Fix: Updated test to expect `"hello-world"` (correct behavior)
- Files modified: crates/memcp-core/src/import/chatgpt.rs
- Commit: e3b11aa

**2. [Rule 2 - Missing] Wired Plan 03 Openclaw/ClaudeCode match arms**
- Found during: Task 2 (build error — non-exhaustive patterns)
- Issue: Plan 03 added variants to `ImportAction` enum but match arms were missing
- Fix: Added Openclaw and ClaudeCode match arms following the same pattern as Jsonl
- Files modified: crates/memcp/src/main.rs
- Commit: 4889715

## Self-Check: PASSED

| Check | Result |
|-|-|
| chatgpt.rs created | FOUND |
| claude_ai.rs created | FOUND |
| markdown.rs created | FOUND |
| curate.rs created | FOUND |
| Commit f7b5265 (Task 1) | FOUND |
| Commit 4889715 (Task 2) | FOUND |
| Commit e3b11aa (test fix) | FOUND |
| cargo build | PASS |
| cargo test (81 passing) | PASS |
| memcp import chatgpt --help | PASS |
| memcp import markdown --help | PASS |
