# Phase 24: Knowledge Tiers - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-17
**Phase:** 24-knowledge-tiers
**Areas discussed:** Tier Assignment, Search Ranking Integration, Source Chain Behavior, Tier Filtering Defaults

---

## Tier Assignment

| Option | Description | Selected |
|--------|-------------|----------|
| A: Fully inferred | Tier always derived from write_path, no caller override | |
| B: Inferred + override | Defaults from write_path, optional knowledge_tier field on CreateMemory | ✓ |
| C: Fully explicit | Caller must always set tier | |

**User's choice:** Option B — inferred by default, caller can override
**Notes:** User specified that `import` should NOT map to `explicit` but should be its own tier value

### Sub-decision: Import tier value

| Option | Description | Selected |
|--------|-------------|----------|
| 1: 5th tier `imported` | New enum value between raw and explicit | ✓ |
| 2: Map to `explicit` | Simpler enum, loses distinction | |
| 3: Map to `raw` | Treats imported as unprocessed | |

**User's choice:** Option 1 — `imported` as a 5th tier
**Notes:** User emphasized import is human-initiated and deserves its own classification

### Sub-decision: source_ids requirement for derived/pattern

| Option | Description | Selected |
|--------|-------------|----------|
| 1: Required for both | Strict provenance | |
| 2: Optional for both | Flexible, trusts caller | |
| 3: Required for derived, optional for pattern | Patterns can emerge from general observation | ✓ |

**User's choice:** Option 3

### Additional decisions
- `auto_store`, `session_summary`, `ingest` (24.5) → raw
- `explicit_store`, `annotation` → explicit
- `import` → imported
- Dreaming (Phase 26) → derived/pattern

---

## Search Ranking Integration

| Option | Description | Selected |
|--------|-------------|----------|
| 1: Multiplicative on salience | Like trust_level: salience * tier_weight | |
| 2: Additive bonus | Flat boost after RRF+salience | |
| 3: Separate scoring dimension | New axis in composite formula: 0.4*RRF + 0.4*(salience*trust) + 0.2*tier_score | ✓ |

**User's choice:** Option 3 — separate scoring dimension
**Notes:** Claude recommended Option 1 (multiplicative) for consistency with trust_level pattern. User chose Option 3 for more explicit tier influence.

---

## Source Chain Behavior

### Sub-decision 1: Deleted/tombstoned sources

| Option | Description | Selected |
|--------|-------------|----------|
| 1: Orphan allowed | Dangling refs, silent | |
| 2: Cascade warning | Tag derived with "orphaned_sources" | ✓ |
| 3: Cascade delete | Delete derived if all sources gone | |

**User's choice:** Option 2 (aligned with recommendation)

### Sub-decision 2: Chain depth

| Option | Description | Selected |
|--------|-------------|----------|
| 1: Single-hop only | Direct sources only | |
| 2: Transitive traversal | Full recursive chain | |
| 3: Single-hop default, deep opt-in | --show-sources=deep for transitive | ✓ |

**User's choice:** Option 3 (aligned with recommendation)

### Sub-decision 3: --show-sources default

| Option | Description | Selected |
|--------|-------------|----------|
| 1: Opt-in | Sources only when --show-sources passed | ✓ |
| 2: Always included | source_ids always in output | |

**User's choice:** Option 1 (aligned with recommendation)

### Additional decisions
- **D-09:** Ingest-created raw memories must have fully populated metadata (UUID, session_id, actor_type, etc.). No hollow memories.

---

## Tier Filtering Defaults

| Option | Description | Selected |
|--------|-------------|----------|
| 1: All tiers (no filter) | Backward compatible, raw included | |
| 2: Exclude raw by default | Search/recall skips raw unless --tier all | ✓ (for query-based) |
| 3: Tier-aware ranking only | All tiers returned, scoring handles it | ✓ (for queryless) |

**User's choice:** Split behavior:
- Query-based search/recall: Option 2 (exclude raw)
- Queryless recall (--first): Option 3 (all tiers, ranked)

**Notes:** User initially asked which would be recommended if Phase 26 (Dreaming) were already done. Claude shifted recommendation from Option 3 to Option 2 for query-based search, noting that with dreaming mature, raw memories are processed input. User agreed with the split approach.

---

## Claude's Discretion

- Migration SQL details and index strategy
- Backfill query logic for existing memories
- Postgres ENUM vs TEXT with check constraint
- Default weight values in memcp.toml
- Orphan detection mechanism

## Deferred Ideas

None — discussion stayed within phase scope
