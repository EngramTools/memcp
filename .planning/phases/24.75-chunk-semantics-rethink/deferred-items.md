# Phase 24.75 Deferred Items

Items discovered during plan execution that are out-of-scope for the current task.

## From Plan 24.75-04

### Pre-existing clippy -D warnings failure

- **Scope:** Codebase-wide. 2052 clippy pedantic warnings in `crates/memcp-core/src/load_test/{metrics,trust}.rs` + 4 in `crates/memcp-core/src/validation.rs`. Unrelated to Plan 04.
- **Evidence:** `cargo clippy --all-targets -- -D warnings` fails on unmodified pre-existing files (e.g. `load_test/trust.rs:796` cast_precision_loss, `load_test/metrics.rs:121` useless_vec, `validation.rs:177` uninlined_format_args).
- **Plan 04's own code:** Adds ~20 pedantic warnings (similar_names on `score`/`store`, doc_markdown on identifiers like `HashMap`, `ChunkingConfig`, `TOPIC_MAX_LEN`). All are `clippy::pedantic` level — equivalent in severity to existing code; no new bug-catchers tripped.
- **Recommendation:** Open a follow-up plan to sweep pedantic warnings workspace-wide (either fix or `#[allow]` at module level). Do not block Plan 04 completion on it — the criteria was broken before my changes.

### No topic-embedding cache persistence

- **Note in plan:** "First call is slow (embedding N spans); no caching required for this phase — flag as a follow-up in SUMMARY if needed."
- **Current behavior:** Span embeddings are NOT cached. Every `get_memory_span` call re-embeds every candidate span. Topic embeddings ARE cached (bounded HashMap on AppState / MemoryService).
- **Recommendation:** Phase 29 (multi-depth summaries) will likely embed spans at store time via the summary pipeline, obsoleting the need. If Phase 29 doesn't land before real usage ramps, add a per-memory span-embedding cache (keyed by memory_id + content hash) bounded to ~1000 entries.
