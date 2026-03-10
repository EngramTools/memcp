---
phase: 09-documentation
plan: 01
subsystem: documentation
tags: [readme, architecture, config, docs]
dependency_graph:
  requires: []
  provides: [README.md, docs/architecture.md, memcp.toml.example]
  affects: [onboarding, developer-experience]
tech_stack:
  added: []
  patterns: [docs-directory-structure]
key_files:
  created:
    - docs/architecture.md
  modified:
    - README.md
    - memcp.toml.example
  deleted:
    - ARCHITECTURE.md
decisions:
  - README slimmed to 102 lines as landing page with links to docs/
  - Architecture doc moved from root to docs/ directory
  - Example config organized by user-facing importance (main vs advanced sections)
metrics:
  duration: ~6min
  completed: "2026-03-10T23:21:13Z"
---

# Phase 09 Plan 01: Documentation Overhaul Summary

Slim README landing page, updated architecture doc in docs/, annotated example config with all sections and env var mappings.

## Tasks Completed

| Task | Name | Commit | Files |
|-|-|-|-|
| 1 | Slim README and create docs/architecture.md | 702dac4 | README.md, docs/architecture.md, ARCHITECTURE.md (deleted) |
| 2 | Create annotated memcp.toml.example | bb95791 | memcp.toml.example |

## What Was Done

### Task 1: Slim README and create docs/architecture.md

Rewrote README.md from 202 lines to 102 lines as a concise landing page: title, badges, one-paragraph description, install instructions, quickstart commands, MCP config, documentation links table, feature list, engram.host mention, license/contributing links.

Moved ARCHITECTURE.md to docs/architecture.md with updates:
- Added Key Subsystems section covering ingestion pipeline (redaction -> filter -> dedup -> embed -> extract -> chunk), retrieval pipeline (temporal -> expansion -> multi-query -> hybrid search -> RRF -> salience -> trust -> rerank), and curation pipeline (cluster -> instruction detection -> priority queue -> LLM review)
- Added redaction/mod.rs and RedactionEngine to pipeline module table
- Updated data flow diagram to include redaction in store path
- Added "Redaction on ingestion" to Key Design Decisions section

Deleted root ARCHITECTURE.md.

### Task 2: Create annotated memcp.toml.example

Complete rewrite of memcp.toml.example with all config sections extracted from config.rs:
- 8 main sections (embedding, search, auto_store, gc, recall, curation, query_intelligence, redaction) with uncommented headers and commented fields
- 18 advanced sections (salience, extraction, consolidation, dedup, chunking, idempotency, temporal, content_filter, summarization, health, resource_caps, store, retention, import, enrichment, rate_limit, observability, promotion, category_filter, user, project) fully commented
- Every field includes its MEMCP__ env var equivalent
- Default values shown inline with type/options documentation

## Deviations from Plan

None -- plan executed exactly as written.

## Verification Results

- README.md is 102 lines (within 80-120 target) and links to docs/ files
- docs/architecture.md covers curation, redaction, chunking, enrichment, trust, recall, promotion, temporal
- memcp.toml.example covers all 8 main config sections with MEMCP__ env var mappings
- No marketing language in any file

## Self-Check: PASSED

- FOUND: README.md (3565 bytes)
- FOUND: docs/architecture.md (12345 bytes)
- FOUND: memcp.toml.example (14662 bytes)
- FOUND: .planning/phases/09-documentation/09-01-SUMMARY.md
- CONFIRMED DELETED: ARCHITECTURE.md
- FOUND commit: 702dac4 (Task 1)
- FOUND commit: bb95791 (Task 2)
