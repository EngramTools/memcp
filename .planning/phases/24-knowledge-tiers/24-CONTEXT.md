# Phase 24: Knowledge Tiers - Context

**Gathered:** 2026-04-17
**Status:** Ready for planning

<domain>
## Phase Boundary

Add `knowledge_tier` enum column (5 values: `raw`, `imported`, `explicit`, `derived`, `pattern`) and `source_ids` JSONB column to the memories table. Tier is inferred from `write_path` by default with optional caller override. Search ranking integrates tier as a separate scoring dimension. Search/recall excludes `raw` by default; queryless recall returns all tiers. Backfill existing memories by `write_path`.

</domain>

<decisions>
## Implementation Decisions

### Tier Assignment
- **D-01:** Inferred by default from `write_path`, caller can override via optional `knowledge_tier` field on `CreateMemory`. Both `CreateMemory` struct and all transport layers (MCP, CLI, HTTP) gain the optional field.
- **D-02:** Default tier mapping:
  - `auto_store`, `session_summary`, `ingest` (Phase 24.5) → `raw`
  - `explicit_store`, `annotation` → `explicit`
  - `import` → `imported`
  - Dreaming (Phase 26) sets `derived` or `pattern` explicitly
- **D-03:** 5-value enum: `raw | imported | explicit | derived | pattern`. `imported` is distinct from `explicit` — it represents human-initiated bulk import from external sources, ranking between raw and explicit.
- **D-04:** `source_ids` required (non-empty) when `knowledge_tier = derived`. Optional (nullable) for `pattern` — patterns can emerge from general observation without specific source references. Null for `raw`, `imported`, `explicit`.

### Search Ranking
- **D-05:** Tier becomes a separate scoring dimension in composite formula: `0.4 * RRF + 0.4 * (salience * trust) + 0.2 * tier_score`. Tier score normalized 0.0-1.0 across the enum (raw=0.0, imported=0.25, explicit=0.5, derived=0.75, pattern=1.0). Weights configurable in `memcp.toml` under `[search.tier_weights]`.

### Source Chain Behavior
- **D-06:** When a source memory is deleted/tombstoned/GC'd, derived memories that reference it get tagged with `"orphaned_sources"`. No cascade delete — derived conclusions are often more valuable than their sources.
- **D-07:** `--show-sources` fetches single-hop (direct source IDs) by default. `--show-sources=deep` walks the full provenance chain recursively. Single-hop covers 95% of use cases; deep traversal is for auditing.
- **D-08:** `--show-sources` is opt-in, not default. Keeps search/recall output token-lean. Agents opt in when debugging or auditing provenance.

### Tier Filtering Defaults
- **D-09:** Ingest-created raw memories (Phase 24.5) must have fully populated metadata — UUID always generated, session_id from request, actor_type from source identifier, project from request context. No hollow memories with just content + UUID.
- **D-10:** Query-based search and recall exclude `raw` tier by default. Callers opt in with `--tier all`, `--tier raw`, or `--tier raw,explicit,...` to include raw. Rationale: with dreaming active (Phase 26), raw memories are processed input — conclusions live in derived/pattern tiers.
- **D-11:** Queryless recall (`--first`, no query) returns all tiers, ranked by salience x tier score. No filtering — cold start needs the full picture since there's no query to rank against.

### Claude's Discretion
- Migration SQL details and index strategy for `knowledge_tier` column
- Exact backfill query logic for classifying existing memories by `write_path` (TIER-03)
- Whether `knowledge_tier` uses a Postgres ENUM type or TEXT with check constraint
- Configurable weight defaults in `memcp.toml` (starting points provided in D-05)
- Orphan detection mechanism — GC hook vs trigger vs periodic sweep

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Memory schema and store layer
- `crates/memcp-core/src/storage/store/mod.rs` — Memory and CreateMemory structs, all fields. New `knowledge_tier` and `source_ids` fields go here.
- `crates/memcp-core/src/storage/store/postgres/queries.rs` — SQL queries for CRUD. Tier column must be threaded through INSERT/SELECT/UPDATE.
- `crates/memcp-core/src/storage/store/postgres/salience.rs` — Salience scoring. Tier boost integrates here per D-05.

### Search and scoring pipeline
- `crates/memcp-core/src/storage/store/postgres/embedding.rs` — `search_similar()`, `rrf_fuse()`, composite scoring. Tier dimension added to scoring formula.
- `crates/memcp-core/src/intelligence/recall/mod.rs` — Recall engine. Tier filtering (D-10, D-11) and `--show-sources` behavior go here.

### Transport layers (MCP, CLI, HTTP)
- `crates/memcp-core/src/transport/server.rs` — MCP tool definitions. `knowledge_tier`, `source_ids`, `--tier` filter, `--show-sources` params.
- `crates/memcp-core/src/cli.rs` — CLI subcommands. `--tier` and `--show-sources` flags on search/recall.
- `crates/memcp-core/src/transport/api/types.rs` — HTTP API request/response types.
- `crates/memcp-core/src/transport/api/store.rs` — HTTP store handler.
- `crates/memcp-core/src/transport/api/search.rs` — HTTP search handler.
- `crates/memcp-core/src/transport/api/recall.rs` — HTTP recall handler.

### Auto-store and ingestion
- `crates/memcp-core/src/pipeline/auto_store/mod.rs` — Auto-store worker. Sets `write_path: "auto_store"` → tier inferred as `raw`.

### Prior phase context
- Phase 23 (Tiered Context Loading) — added `abstract_text`, `overview_text`, `abstraction_status`. Similar schema extension pattern.
- Phase 11.1 (Provenance Tagging) — added `trust_level`, `write_path`, `session_id`, `agent_role`. Tier inference depends on `write_path`.

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `write_path` field already discriminates creation paths (`auto_store`, `explicit_store`, `session_summary`, `annotation`, `import`) — tier inference maps directly from this
- `trust_level` multiplicative scoring in `salience.rs` — same pattern for tier scoring, but D-05 uses a separate dimension instead
- `SearchFilter` struct — already supports field-based filtering, extend with `tier` filter
- `embedding_tier` column (Phase 08.7) — existing enum column pattern for reference, though this is for embedding quality tiers not knowledge tiers

### Established Patterns
- Schema migrations use sequential numbered files in `migrations/`
- New columns on `Memory` struct follow the pattern: add to struct, add to `CreateMemory`, thread through SQL, add to MCP/CLI/HTTP
- Optional fields on `CreateMemory` use `#[serde(default)]` with `Option<T>`
- GC worker in `pipeline/` already handles memory cleanup — orphan tagging hooks in here

### Integration Points
- `store()` in `postgres/mod.rs` — tier inference logic goes here (read `write_path`, map to tier, allow override)
- `search_similar()` in `embedding.rs` — composite scoring formula change (D-05)
- `recall()` and `recall_queryless()` in `recall/mod.rs` — tier filtering split (D-10 vs D-11)
- GC worker — orphan source detection and tagging (D-06)
- All 3 transport layers (MCP server.rs, CLI cli.rs, HTTP api/) — new params

</code_context>

<specifics>
## Specific Ideas

- Favorite color example validates the design: raw "color is blue" + raw "color is red" → dreaming creates derived "color is red (supersedes blue)" with `source_ids` → old blue memory tombstoned via stability drop → derived memory surfaces first in search via tier boost
- `imported` tier slots between raw and explicit in ranking — imported memories are curated by the user but from external sources, not firsthand agent knowledge
- Phase 24.5's `ingest` endpoint creates `raw` memories with full metadata — no hollow memories. This is the HTTP push equivalent of auto-store's file-watching path.

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 24-knowledge-tiers*
*Context gathered: 2026-04-17*
