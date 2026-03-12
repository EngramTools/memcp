---
phase: 23-tiered-context-loading
plan: "01"
subsystem: pipeline/abstraction + storage/store + config
tags: [tiered-context, abstraction, pipeline, schema, config]
dependency_graph:
  requires: ["23-00"]
  provides: ["AbstractionProvider trait", "AbstractionConfig", "023 migration", "Memory tiered fields"]
  affects: ["storage/store/postgres.rs", "config.rs", "pipeline/mod.rs"]
tech_stack:
  added: ["pipeline/abstraction module", "AbstractionProvider trait", "OllamaAbstractionProvider", "OpenAIAbstractionProvider"]
  patterns: ["async_trait provider pattern (mirrors SummarizationProvider)", "opt-in config with serde defaults"]
key_files:
  created:
    - crates/memcp-core/migrations/023_tiered_content.sql
    - crates/memcp-core/src/pipeline/abstraction/mod.rs
    - crates/memcp-core/src/pipeline/abstraction/ollama.rs
    - crates/memcp-core/src/pipeline/abstraction/openai.rs
  modified:
    - crates/memcp-core/src/storage/store/mod.rs
    - crates/memcp-core/src/storage/store/postgres.rs
    - crates/memcp-core/src/config.rs
    - crates/memcp-core/src/pipeline/mod.rs
    - crates/memcp-core/src/lib.rs
    - crates/memcp-core/tests/unit/abstraction.rs
decisions:
  - "Short content (<200 chars) set to abstraction_status='skipped' at store time — computed before INSERT and passed as param, reused in returned Memory struct"
  - "row_to_memory uses .unwrap_or for all three new fields for backward compat with pre-migration rows"
  - "AbstractionConfig URL validation wired into validate_provider_urls() alongside existing providers"
  - "Two max_tokens tiers: abstract=150 (L0), overview=600 (L1) for OpenAI"
metrics:
  duration: "~35 minutes"
  completed_date: "2026-03-12"
  tasks_completed: 2
  tasks_total: 2
  files_created: 4
  files_modified: 6
---

# Phase 23 Plan 01: Tiered Content Schema, Types, Config, and AbstractionProvider Summary

Foundation for L0/L1/L2 tiered memory representation: migration, Memory struct fields, AbstractionConfig, and AbstractionProvider trait with Ollama and OpenAI implementations.

## What Was Built

### Task 1: Migration + Memory Struct + AbstractionConfig

**Migration 023** (`023_tiered_content.sql`):
- `abstract_text TEXT` — L0 abstract, ~100 tokens, for semantic search
- `overview_text TEXT` — L1 overview, ~500 tokens, mid-level context
- `abstraction_status TEXT NOT NULL DEFAULT 'pending'` — "pending" | "complete" | "failed" | "skipped"
- Index on pending status for efficient worker polling

**Memory struct** (`storage/store/mod.rs`): three new fields added with doc comments explaining each tier.

**postgres.rs**: All `row_to_memory` calls use `.unwrap_or` for backward compat. INSERT includes `abstraction_status` explicitly — "skipped" for content < 200 chars, "pending" otherwise. All SELECT queries returning Memory include the three new columns.

**AbstractionConfig** (`config.rs`): Full config struct mirroring SummarizationConfig pattern — `enabled`, `provider`, `generate_overview`, `abstract_prompt_template`, `overview_prompt_template`, Ollama and OpenAI settings, `max_input_chars`, `min_content_length`. Default prompt templates baked in. Registered in root `Config` with `#[serde(default)]`. URL validation wired into `validate_provider_urls()`.

**count_pending_abstractions()**: New method on PostgresMemoryStore for status/metrics.

### Task 2: AbstractionProvider Trait + Implementations + Tests

**pipeline/abstraction/mod.rs**: `AbstractionError` (Generation, Api, NotConfigured), `AbstractionProvider` trait with `generate_abstract()`, `generate_overview()`, `model_name()`, and `create_abstraction_provider()` factory.

**ollama.rs**: Calls `/api/chat` with separate system prompts for abstract vs overview. Template `{content}` placeholder replaced with truncated content.

**openai.rs**: Calls `/chat/completions` with tier-specific `max_tokens` (150 for L0, 600 for L1).

**Unit tests**: 3 provider creation tests pass (disabled → None, ollama → Some, openai missing key → Err).

## Decisions Made

1. Short content (<200 chars) gets `abstraction_status='skipped'` at store time — abstraction adds no value for brief memories. The threshold matches `AbstractionConfig.min_content_length` default.
2. The `abstraction_status` variable is computed before the INSERT query and reused in the returned `Memory` struct to avoid a borrow-after-move issue on `input.content`.
3. `row_to_memory` uses `.unwrap_or` for all three new fields — any query not including these columns (pre-migration rows or JOIN queries) gets sensible defaults without error.
4. AbstractionConfig URL validation is wired into the existing `validate_provider_urls()` method for consistency with all other provider configs.

## Deviations from Plan

None — plan executed exactly as written. The only notable implementation detail was the borrow-after-move fix in the store() method, which required computing `abstraction_status` as a local variable before building the SQL query and then the returned Memory.

## Self-Check: PASSED

All created files exist on disk. All task commits verified in git log.
