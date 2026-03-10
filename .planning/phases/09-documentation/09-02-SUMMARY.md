---
phase: 09-documentation
plan: 02
title: "Config & CLI Reference Documentation"
subsystem: documentation
tags: [docs, config, cli, reference]
dependency_graph:
  requires: []
  provides: [config-reference, cli-reference]
  affects: [user-onboarding]
tech_stack:
  added: []
  patterns: [markdown-tables, env-var-documentation]
key_files:
  created:
    - docs/config-reference.md
    - docs/cli-reference.md
  modified: []
decisions:
  - Organized config sections to mirror TOML structure (embedding, search, salience, etc.)
  - Included full env var appendix for quick scanning (120+ vars sorted alphabetically)
  - Grouped CLI subcommands logically rather than alphabetically for usability
  - Documented nested subcommands inline (embed, curation, import, statusline, daemon)
metrics:
  duration: ~10m
  completed: "2026-03-10T23:24:45Z"
---

# Phase 09 Plan 02: Config & CLI Reference Documentation Summary

Comprehensive config reference and CLI reference derived from source code -- 35+ config structs, 120+ env vars, 28+ CLI subcommands with full option tables and examples.

## What Was Built

### Config Reference (docs/config-reference.md, 696 lines)

- Documented all 35 config structs from `crates/memcp-core/src/config.rs`
- Every field has: TOML key, env var, Rust type, default value, description
- Covers nested structs: embedding tiers (routing, promotion), category filter, redaction (allowlist, custom rules)
- Includes figment loading order explanation and DATABASE_URL special case
- Appendix with all 120+ environment variables sorted alphabetically for quick scanning

### CLI Reference (docs/cli-reference.md, 719 lines)

- Documented all 28 CLI subcommands with synopsis, option tables, and concrete examples
- Nested subcommands fully covered: embed (3), curation (3), import (9), statusline (2), daemon (1)
- Grouped logically: Core Operations, Memory Management, Discovery, Import/Export, Daemon & Server, Maintenance, Embedding Management, Status Line
- Common import options factored into shared table with source-specific options below

## Task Completion

| Task | Name | Commit | Files |
|-|-|-|-|
| 1 | Create config reference | cc9f384 | docs/config-reference.md |
| 2 | Create CLI reference | 1df6a03 | docs/cli-reference.md |

## Deviations from Plan

None -- plan executed exactly as written.

## Self-Check: PASSED

- FOUND: docs/config-reference.md
- FOUND: docs/cli-reference.md
- FOUND: cc9f384 (config reference commit)
- FOUND: 1df6a03 (CLI reference commit)
