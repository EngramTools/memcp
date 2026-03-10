# MCP Tools Reference

memcp exposes tools via the Model Context Protocol (MCP) when running in serve mode (`memcp serve`). These tools are available over stdio transport for direct integration with MCP-compatible clients.

## Tool Discovery

Tool descriptions serve as the **primary discovery mechanism** for AI models. Each tool's description contains enough information for a model to use it correctly without system prompt hints. Parameter names, types, and descriptions are derived from the Rust struct schemas via `schemars::JsonSchema`.

## Integer References

memcp uses a **UuidRefMap** system to prevent UUID hallucination errors. When tools return memory objects, each includes both an `id` (UUID) and a `ref` (sequential integer like 1, 2, 3). In subsequent calls, you can pass either the UUID or the integer ref as the `id` parameter -- memcp resolves both.

- Refs are session-scoped: they reset between MCP connections.
- The same UUID always maps to the same ref within a session.
- Integer refs start from 1 (more natural for agents).

## Sandbox Access (CEX-03)

Tools annotated with `allowed_callers: ["direct", "code_execution_20260120"]` can be invoked from code execution sandboxes. Currently sandbox-safe tools are: `store_memory`, `search_memory`, `recall_memory`, `annotate_memory`. Destructive tools (`delete_memory`, `bulk_delete_memories`) are intentionally excluded.

---

## Tools

### store_memory

Store a new memory. Deduplication: identical content within the server dedup window returns the existing memory. Optional `idempotency_key` for caller-controlled at-most-once semantics.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-|-|-|-|-|
| content | string | yes | - | Memory content to store |
| type_hint | string | no | "fact" | Classification: fact, preference, instruction, decision |
| source | string | no | "mcp" | Origin source identifier |
| tags | string[] | no | null | Tags for categorization |
| actor | string | no | null | Actor identity |
| actor_type | string | no | "agent" | Actor type: agent, human, system |
| audience | string | no | "global" | Scope: global, personal, team:X |
| idempotency_key | string | no | null | At-most-once key (first wins) |
| wait | bool | no | false | Block until embedding completes |
| trust_level | float | no | auto | Trust 0.0-1.0. Omit to auto-infer |
| session_id | string | no | null | Groups memories by conversation |
| agent_role | string | no | null | Agent's role (coder, reviewer, planner) |
| write_path | string | no | null | How created: session_summary, explicit_store, annotation, import |
| skip_redaction | bool | no | false | Bypass secret/PII redaction |

**Returns:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "ref": 1,
  "content": "User prefers dark mode",
  "type_hint": "preference",
  "source": "mcp",
  "tags": ["ui"],
  "created_at": "2026-03-10T12:00:00Z",
  "updated_at": "2026-03-10T12:00:00Z",
  "access_count": 0,
  "embedding_status": "pending",
  "actor": null,
  "actor_type": "agent",
  "audience": "global",
  "hint": "Use get_memory with this ID to retrieve, or update_memory to modify"
}
```

When `wait: true` and embedding completes: `embedding_status` becomes `"complete"` and `embedding_dimension` is included.

When content is redacted: `redactions: {"count": 1, "categories": ["aws_key"]}` is included.

When approaching resource caps: `warning: "Memory usage at 85%..."` is included.

**Example call:**
```json
{
  "name": "store_memory",
  "arguments": {
    "content": "User prefers dark mode in all editors",
    "tags": ["ui", "preference"],
    "type_hint": "preference"
  }
}
```

---

### get_memory

Get a full memory by ID. Implicitly bumps salience on direct retrieval.

**Parameters:**

| Parameter | Type | Required | Description |
|-|-|-|-|
| id | string | yes | Memory ID (UUID or integer ref) |

**Returns:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "ref": 1,
  "content": "User prefers dark mode",
  "type_hint": "preference",
  "source": "mcp",
  "tags": ["ui"],
  "created_at": "2026-03-10T12:00:00Z",
  "updated_at": "2026-03-10T12:00:00Z",
  "last_accessed_at": "2026-03-10T14:00:00Z",
  "access_count": 3,
  "embedding_status": "complete",
  "actor": null,
  "actor_type": "agent",
  "audience": "global",
  "hint": "Use update_memory to modify or delete_memory to remove"
}
```

**Example call:**
```json
{
  "name": "get_memory",
  "arguments": {"id": "1"}
}
```

---

### search_memory

Search memories by meaning. Returns salience-ranked results using hybrid search (BM25 + vector + symbolic) with RRF fusion.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-|-|-|-|-|
| query | string | yes | - | Search query |
| limit | integer | no | 20 | Max results (1-100, capped by resource_caps.max_search_results) |
| tags | string[] | no | null | Filter: all must match |
| audience | string | no | null | Filter by audience |
| project | string | no | null | Project scope (returns project + global memories) |
| min_salience | float | no | config default | Minimum salience score (0.0-1.0) |
| fields | string[] | no | null | Field projection (e.g., ["id","content"]). Supports one-level dot-notation |
| cursor | string | no | null | Pagination token from previous next_cursor |
| created_after | string | no | null | ISO-8601 datetime filter |
| created_before | string | no | null | ISO-8601 datetime filter |
| bm25_weight | float | no | 1.0 | BM25 leg weight (0=disable, >1=emphasize) |
| vector_weight | float | no | 1.0 | Vector leg weight (0=disable, >1=emphasize) |
| symbolic_weight | float | no | 1.0 | Symbolic leg weight (0=disable, >1=emphasize) |

At least one search leg must be enabled (non-zero weight).

**Returns:**
```json
{
  "memories": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "ref": 1,
      "content": "User prefers dark mode",
      "type_hint": "preference",
      "source": "mcp",
      "tags": ["ui"],
      "created_at": "2026-03-10T12:00:00Z",
      "updated_at": "2026-03-10T12:00:00Z",
      "access_count": 3,
      "relevance_score": 0.85,
      "composite_score": 0.92,
      "match_source": "hybrid",
      "rrf_score": 0.016,
      "actor": null,
      "actor_type": "agent",
      "audience": "global"
    }
  ],
  "total_results": 1,
  "query": "dark mode preferences",
  "next_cursor": null,
  "has_more": false
}
```

`composite_score` is a 0-1 blended relevance combining retrieval similarity (RRF) and trust-weighted memory importance (salience).

When query intelligence is enabled, may include `decomposed: true` and `sub_queries: [...]`.

When no results match and salience hint mode is on: `salience_hint: "3 results found below threshold 0.3"`.

With `fields: ["id", "content"]`: each result has only `{"id": "uuid", "ref": 1, "content": "text"}`.

**Example call:**
```json
{
  "name": "search_memory",
  "arguments": {
    "query": "what are the user's UI preferences?",
    "limit": 5,
    "tags": ["preference"]
  }
}
```

---

### recall_memory

Recall relevant memories for automatic context injection. Supports query-based (vector similarity) and queryless (salience-ranked cold-start) modes.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-|-|-|-|-|
| query | string | no | null | Query text. Omit for queryless cold-start |
| session_id | string | no | auto-generated | Session ID for dedup tracking |
| reset | bool | no | false | Clear session recall history |
| first | bool | no | false | Session-start mode: pins project-summary, adds preamble/datetime |
| limit | integer | no | config default | Override max_memories |
| boost_tags | string[] | no | null | Tag affinity boost. Prefix matching: "channel:" boosts all channel:* |

**Returns:**
```json
{
  "session_id": "sess_abc123",
  "count": 2,
  "memories": [
    {
      "memory_id": "550e8400-e29b-41d4-a716-446655440000",
      "ref": 1,
      "content": "User prefers dark mode",
      "relevance": 0.92
    }
  ],
  "summary": {
    "memory_id": "660e9500-f30c-52e5-b827-557766551111",
    "ref": 2,
    "content": "Project summary..."
  }
}
```

The `summary` field is present when a project-summary memory exists and `first: true`.

When `boost_tags` match, memories include `boost_applied: true` and `boost_score`.

Session-scoped dedup prevents re-injection of the same memory within a conversation.

**Example call (queryless cold-start):**
```json
{
  "name": "recall_memory",
  "arguments": {
    "first": true
  }
}
```

**Example call (query-based):**
```json
{
  "name": "recall_memory",
  "arguments": {
    "query": "what UI preferences does the user have?",
    "session_id": "sess_abc123"
  }
}
```

---

### update_memory

Update a memory's content or metadata. At least one field (besides id) is required.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-|-|-|-|-|
| id | string | yes | - | Memory ID (UUID or integer ref) |
| content | string | no | null | New content (triggers re-embedding and re-extraction) |
| type_hint | string | no | null | New classification |
| source | string | no | null | New source |
| tags | string[] | no | null | New tags (replaces existing) |
| trust_level | float | no | null | Trust level override 0.0-1.0 (JSONB audit trail) |

Content changes trigger embedding re-queuing. Tag-only changes skip re-embedding by default (configurable via `embedding.reembed_on_tag_change`). Content filter is applied when content changes.

**Returns:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "ref": 1,
  "content": "Updated: user prefers dark mode with high contrast",
  "type_hint": "preference",
  "source": "mcp",
  "tags": ["ui", "accessibility"],
  "created_at": "2026-03-10T12:00:00Z",
  "updated_at": "2026-03-10T15:00:00Z",
  "access_count": 3,
  "embedding_status": "pending",
  "actor": null,
  "actor_type": "agent",
  "audience": "global",
  "hint": "Use get_memory to re-read or delete_memory to remove"
}
```

**Example call:**
```json
{
  "name": "update_memory",
  "arguments": {
    "id": "1",
    "content": "User prefers dark mode with high contrast in all editors",
    "tags": ["ui", "preference", "accessibility"]
  }
}
```

---

### delete_memory

Delete a memory by ID. Idempotent: returns success even if the memory does not exist (safe to retry).

**Parameters:**

| Parameter | Type | Required | Description |
|-|-|-|-|
| id | string | yes | Memory ID (UUID or integer ref) |

**Returns:**
```json
{
  "deleted": true,
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "hint": "Memory permanently removed. Use store_memory to create new memories."
}
```

**Example call:**
```json
{
  "name": "delete_memory",
  "arguments": {"id": "1"}
}
```

---

### annotate_memory

Annotate an existing memory -- add/replace tags and adjust salience. Returns a diff showing changes.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-|-|-|-|-|
| id | string | yes | - | Memory ID (UUID or integer ref) |
| tags | string[] | no | null | Tags to append (merged, deduplicated) |
| replace_tags | string[] | no | null | Tags to replace ALL existing tags |
| salience | string | no | null | Absolute ("0.9") or multiplier ("1.5x") for stability |

**Returns:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "ref": 1,
  "changes": {
    "tags_added": ["important", "decision"],
    "tags_removed": [],
    "salience_before": 2.5,
    "salience_after": 3.75
  }
}
```

Salience fields only appear when salience was modified.

**Example call:**
```json
{
  "name": "annotate_memory",
  "arguments": {
    "id": "1",
    "tags": ["important", "decision"],
    "salience": "1.5x"
  }
}
```

---

### discover_memories

Discover unexpected connections between memories. Finds memories in the cosine similarity sweet spot (default 0.3-0.7) -- related enough to be meaningful but different enough to be surprising. Use for creative exploration and lateral thinking, not for finding specific information (use `search_memory` for that).

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-|-|-|-|-|
| query | string | yes | - | Topic or concept to explore |
| min_similarity | float | no | 0.3 | Lower bound (0.0-1.0). Lower = more surprising |
| max_similarity | float | no | 0.7 | Upper bound (0.0-1.0). Higher = more obvious |
| limit | integer | no | 10 | Max results (1-50) |
| project | string | no | null | Project scope filter |

**Returns:**
```json
{
  "discoveries": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "ref": 1,
      "content": "Memory about design patterns...",
      "type_hint": "fact",
      "tags": ["architecture"],
      "similarity": "0.452",
      "created_at": "2026-03-08T10:30:00Z",
      "connection": "Both relate to organizing complex systems..."
    }
  ],
  "query": "code organization",
  "similarity_range": [0.3, 0.7],
  "count": 1
}
```

The `connection` field contains LLM-generated explanations when query intelligence is enabled (fail-open: absent if unavailable).

**Example call:**
```json
{
  "name": "discover_memories",
  "arguments": {
    "query": "code organization",
    "min_similarity": 0.25,
    "max_similarity": 0.65,
    "limit": 5
  }
}
```

---

### feedback_memory

Provide relevance feedback for a memory (useful or irrelevant). Adjusts salience scoring for future retrieval.

**Parameters:**

| Parameter | Type | Required | Description |
|-|-|-|-|
| id | string | yes | Memory ID (UUID or integer ref) |
| signal | string | yes | Feedback signal: "useful" or "irrelevant" |

**Returns:**
```json
{"ok": true}
```

**Example call:**
```json
{
  "name": "feedback_memory",
  "arguments": {
    "id": "1",
    "signal": "useful"
  }
}
```

---

## Additional Tools

These tools are available via MCP but are less commonly used by agents:

### list_memories

List memories with filters and cursor-based pagination.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-|-|-|-|-|
| type_hint | string | no | null | Filter by type_hint |
| source | string | no | null | Filter by source |
| created_after | string | no | null | ISO-8601 datetime |
| created_before | string | no | null | ISO-8601 datetime |
| updated_after | string | no | null | ISO-8601 datetime |
| updated_before | string | no | null | ISO-8601 datetime |
| limit | integer | no | 20 | Max results (1-100) |
| cursor | string | no | null | Pagination cursor |
| actor | string | no | null | Filter by actor |
| audience | string | no | null | Filter by audience |
| session_id | string | no | null | Filter by session |
| agent_role | string | no | null | Filter by agent role |

**Returns:**
```json
{
  "memories": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "ref": 1,
      "content": "...",
      "type_hint": "fact",
      "source": "mcp",
      "tags": [],
      "created_at": "2026-03-10T12:00:00Z",
      "updated_at": "2026-03-10T12:00:00Z",
      "access_count": 0,
      "embedding_status": "complete",
      "actor": null,
      "actor_type": "agent",
      "audience": "global"
    }
  ],
  "count": 1,
  "next_cursor": null,
  "has_more": false,
  "hint": "Use next_cursor value in cursor parameter to get next page"
}
```

**Example call:**
```json
{
  "name": "list_memories",
  "arguments": {
    "type_hint": "decision",
    "limit": 10
  }
}
```

### bulk_delete_memories

Bulk delete by filter. Two-phase: `confirm=false` returns count, `confirm=true` deletes.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-|-|-|-|-|
| type_hint | string | no | null | Filter by type_hint |
| source | string | no | null | Filter by source |
| created_after | string | no | null | ISO-8601 datetime |
| created_before | string | no | null | ISO-8601 datetime |
| updated_after | string | no | null | ISO-8601 datetime |
| updated_before | string | no | null | ISO-8601 datetime |
| confirm | bool | no | false | false=count only, true=actually delete |

**Returns (confirm=false):**
```json
{
  "matched": 5,
  "deleted": false,
  "hint": "Call bulk_delete_memories again with confirm: true to delete these 5 memories"
}
```

**Returns (confirm=true):**
```json
{
  "deleted": 5,
  "confirmed": true,
  "hint": "Bulk deletion complete. Use list_memories to verify."
}
```

**Example call:**
```json
{
  "name": "bulk_delete_memories",
  "arguments": {
    "source": "auto-store",
    "created_before": "2026-02-01T00:00:00Z",
    "confirm": false
  }
}
```

### reinforce_memory

Reinforce a memory to boost future search salience using spaced repetition (FSRS).

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-|-|-|-|-|
| id | string | yes | - | Memory ID (UUID or integer ref) |
| rating | string | no | "good" | Strength: "good" or "easy" |

**Returns:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "ref": 1,
  "stability": 12.5,
  "reinforcement_count": 3,
  "message": "Memory reinforced. Stability: 12.5 days, reinforcements: 3"
}
```

**Example call:**
```json
{
  "name": "reinforce_memory",
  "arguments": {"id": "1", "rating": "good"}
}
```

### health_check

Server health check.

**Returns:**
```json
{
  "status": "ok",
  "version": "0.1.0",
  "uptime_seconds": 3600
}
```

**Example call:**
```json
{"name": "health_check", "arguments": {}}
```
