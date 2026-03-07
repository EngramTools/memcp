---
phase: 14-memory-boosting
plan: 03
subsystem: search
tags: [query-intelligence, rrf, multi-query, decomposition, ollama, openai, rust]

requires:
  - phase: 14-01
    provides: UUID hallucination prevention (session refs) used by search_memory handler

provides:
  - DecomposedQuery type with is_multi_faceted/sub_queries/variants/time_range fields
  - decompose() trait method on QueryIntelligenceProvider (default falls back to expand())
  - build_decomposition_prompt() and decomposition_schema() for LLM structured output
  - decompose() implementation in OllamaQueryIntelligenceProvider and OpenAIQueryIntelligenceProvider
  - rrf_fuse_multi() for merging results from multiple sub-query searches
  - Multi-query pipeline in search_memory handler (decompose → embed sub-queries → fuse → salience rank)
  - multi_query_enabled config flag in QueryIntelligenceConfig (default true)
  - Debug metadata in search response (decomposed, sub_queries fields)

affects: [benchmarking, search quality evaluation, query intelligence tuning]

tech-stack:
  added: []
  patterns:
    - "Fail-open decomposition: on LLM/parse failure return raw query as sole variant"
    - "Multi-query RRF: embed each sub-query independently, fuse by rank accumulation"
    - "decompose() replaces expand() as primary QI method; expand() retained for backward compat"
    - "Debug metadata injected into search response when decomposition occurs"

key-files:
  created: []
  modified:
    - crates/memcp-core/src/intelligence/query_intelligence/mod.rs
    - crates/memcp-core/src/intelligence/query_intelligence/ollama.rs
    - crates/memcp-core/src/intelligence/query_intelligence/openai.rs
    - crates/memcp-core/src/intelligence/search/mod.rs
    - crates/memcp-core/src/transport/server.rs
    - crates/memcp-core/src/config.rs

key-decisions:
  - "decompose() added as optional trait method with default fallback to expand() for backward compat"
  - "Multi-query path uses rrf_fuse_multi with k=60.0 (research default) across sub-query legs"
  - "get_memories_by_ids used post-fusion to fetch full Memory objects in fused rank order"
  - "decomposed/sub_queries debug metadata always injected in response (not debug-only flag)"
  - "multi_query_enabled defaults to true; false disables multi-faceted path even if LLM says is_multi_faceted"

patterns-established:
  - "Fail-open pattern: all LLM-path code returns usable fallback on error, never fails hard"
  - "Sub-query embedding is sequential (not batched) — acceptable for 2-4 queries at QI latency budget"
  - "rrf_fuse_multi returns Vec<(id, score)> without match_source — multi_query used as source label"

requirements-completed: [MQ-01, MQ-02, MQ-03]

duration: 12min
completed: 2026-03-07
---

# Phase 14 Plan 03: Multi-Query Retrieval Summary

**Query decomposition via LLM replacing single-query expansion: DecomposedQuery type, decompose() on QI providers, rrf_fuse_multi() for sub-query result fusion, multi-query pipeline in search_memory handler**

## Performance

- **Duration:** 12 min
- **Started:** 2026-03-07T06:08:27Z
- **Completed:** 2026-03-07T06:20:27Z
- **Tasks:** 3
- **Files modified:** 6

## Accomplishments
- Added `DecomposedQuery` type with `is_multi_faceted`, `sub_queries`, `variants`, `time_range` fields; unified type for both simple and multi-faceted query analysis
- Added `decompose()` to `QueryIntelligenceProvider` trait with default fallback to `expand()` for backward compat; both Ollama and OpenAI providers implement it using `decomposition_schema()` structured output
- Added `rrf_fuse_multi()` to `intelligence/search/mod.rs` for merging multiple sub-query ranked lists via RRF — memories appearing in multiple sub-queries accumulate score across legs
- Wired multi-query pipeline into `search_memory` handler: decompose → embed sub-queries → parallel hybrid_search → rrf_fuse_multi → fetch by ID → salience rank; falls back to single-query on all-legs-fail
- Added `multi_query_enabled` config flag (default true) to `QueryIntelligenceConfig`
- Added debug metadata (`decomposed`, `sub_queries`) to search response JSON

## Task Commits

1. **Task 1: Define DecomposedQuery type, decomposition prompt, and decompose() trait method** - `dffe05d` (feat+test)
2. **Task 2: Implement decompose() in Ollama and OpenAI providers** - `f8c6a30` (feat)
3. **Task 3: Wire multi-query pipeline + rrf_fuse_multi** - `48f2711` (feat)

## Files Created/Modified
- `crates/memcp-core/src/intelligence/query_intelligence/mod.rs` - DecomposedQuery type, build_decomposition_prompt(), decomposition_schema(), decompose() default impl, unit tests
- `crates/memcp-core/src/intelligence/query_intelligence/ollama.rs` - decompose() implementation with fail-open error handling
- `crates/memcp-core/src/intelligence/query_intelligence/openai.rs` - decompose() implementation with fail-open error handling
- `crates/memcp-core/src/intelligence/search/mod.rs` - rrf_fuse_multi() function with 3 unit tests
- `crates/memcp-core/src/transport/server.rs` - Multi-query pipeline in search_memory handler, debug metadata in response
- `crates/memcp-core/src/config.rs` - multi_query_enabled field in QueryIntelligenceConfig

## Decisions Made
- Used `get_memories_by_ids` (existing) post-fusion rather than maintaining HybridRawHit throughout the multi-query path — simpler and avoids duplicating the complex hybrid_search return type across sub-queries
- `decompose()` default impl wraps `expand()` so all existing providers continue to work without change
- Debug metadata (`decomposed`, `sub_queries`) injected unconditionally in response — not gated on a debug flag, since it's lightweight and useful for observing behavior in production

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

- `get_memories_by_ids` returns `HashMap<String, Memory>` (not `Vec<Memory>`) — adjusted post-fusion collection logic to iterate `fused_ids` in order and look up from the map, preserving rank order.

## Next Phase Readiness
- Multi-query decomposition is live and gated by `multi_query_enabled` (default on)
- Ready for benchmark evaluation: run LoCoMo benchmark with multi_query_enabled=true vs false to measure recall improvement
- Future: consider batch embedding for sub-queries to reduce latency on 4-sub-query decompositions

---
*Phase: 14-memory-boosting*
*Completed: 2026-03-07*
