---
phase: 09-documentation
plan: 04
subsystem: qa
tags: [documentation, qa, testing, yaml]
dependency_graph:
  requires: [09-01, 09-02, 09-03]
  provides: [qa-playbook, agent-test-cases]
  affects: [docs, qa]
tech_stack:
  added: []
  patterns: [yaml-test-schema, isolated-qa-database]
key_files:
  created:
    - qa/agent/schema.yaml
    - qa/agent/config.yaml
    - qa/agent/core.yaml
    - qa/agent/gc.yaml
    - qa/agent/curation.yaml
    - qa/agent/import-export.yaml
    - qa/agent/trust.yaml
    - docs/qa-playbook.md
  modified: []
decisions:
  - "YAML schema with 8 assertion types covers CLI testing without custom parser"
  - "QA isolated on port 5434 (dev=5433, system=5432) with Docker lifecycle hooks"
  - "Curation tests use CLI subcommands (run/log/undo) since quarantine is internal pipeline"
metrics:
  duration: ~4min
  completed: "2026-03-10T23:27:00Z"
  tasks_completed: 2
  tasks_total: 2
requirements: [DOC-08, DOC-09, DOC-10]
---

# Phase 09 Plan 04: QA Playbook and Agent Test Cases Summary

Dual-format QA suite: 46 YAML test cases for autonomous agent execution and a human-readable playbook with 5 step-by-step journeys, all running against an isolated Postgres on port 5434.

## What Was Done

### Task 1: QA YAML Schema and Runner Config
- **schema.yaml**: Defines test case structure with 8 assertion types (exit_code, json_path, stdout_contains, stdout_not_contains, stderr_contains, regex, file_exists, file_contains)
- **config.yaml**: Isolated Docker Postgres on port 5434, three run modes (smoke/standard/exhaustive), lifecycle hooks for container management and migrations
- Commit: `c505814`

### Task 2: YAML Test Cases and Human Playbook
- **core.yaml** (22 tests): Full store/search/recall/feedback/list/get/delete/annotate/update cycle with dependency chains
- **gc.yaml** (6 tests): GC dry-run, threshold override, prune execution, dedup detection, reinforce
- **curation.yaml** (6 tests): Curation propose/run/log/undo and status check
- **import-export.yaml** (6 tests): JSONL/CSV/Markdown export, project-filtered export, round-trip import, discover
- **trust.yaml** (6 tests): High/low/default trust storage, search ranking verification, recall path, verbose metadata
- **qa-playbook.md**: 5 human journeys (store-search-recall, feedback loop, GC flow, import/export round-trip, daemon lifecycle) plus quick smoke test
- Commit: `cbedb50`

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing functionality] Curation tests adapted for actual CLI surface**
- **Found during:** Task 2
- **Issue:** Plan specified quarantine/unquarantine CLI commands that don't exist (quarantine is internal to curation pipeline)
- **Fix:** Replaced with curation CLI subcommands (run --propose, run, log, undo) which are the actual CLI surface for curation features
- **Files modified:** qa/agent/curation.yaml

## Verification

- All 7 YAML files and playbook created in correct locations
- 46 total test cases across 5 category files
- Every test has at least one assertion
- No test references dev database port 5433 in commands (only in config comment explaining port choice)
- Schema supports all assertion types needed for CLI testing

## Self-Check: PASSED

- All 9 files found on disk
- Commit c505814 verified in git log
- Commit cbedb50 verified in git log
