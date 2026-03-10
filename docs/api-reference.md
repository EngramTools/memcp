# HTTP API Reference

Base URL: `http://localhost:9090` (configurable via `[daemon] port` in memcp.toml)

Authentication: None (local mode). Token-based auth planned for engram.host.

All request bodies are JSON (`Content-Type: application/json`). All responses are JSON unless otherwise noted.

Common error shape:
```json
{"error": "description of what went wrong"}
```

Rate limiting: When enabled, all `/v1/*` endpoints return `429 Too Many Requests` with:
```json
{"error": "rate limited", "retry_after_ms": 1000}
```
The `Retry-After` header is also set.

---

## Endpoints

### POST /v1/store

Store a new memory. Explicit API stores get salience stability=3.0 (stronger signal than auto-store).

**Request Body:**

| Field | Type | Required | Default | Description |
|-|-|-|-|-|
| content | string | yes | - | Memory content |
| type_hint | string | no | "fact" | Classification: fact, preference, instruction, decision, summary |
| source | string | no | "api" | Provenance identifier |
| tags | string[] | no | null | Tags for categorization and retrieval |
| actor | string | no | null | Actor identity (agent name, user, etc.) |
| actor_type | string | no | "agent" | Actor type: agent, user, system |
| audience | string | no | "global" | Scope: global, personal, team:X |
| idempotency_key | string | no | null | At-most-once semantics -- same key always returns original result |
| wait | bool | no | false | When true, blocks until embedding completes (or sync_timeout_secs) |
| project | string | no | null | Project scope for this memory |
| trust_level | float | no | auto | Trust level 0.0-1.0. Omit to auto-infer from source/actor_type |
| session_id | string | no | null | Groups memories by conversation session |
| agent_role | string | no | null | Agent's role (e.g., coder, reviewer, planner) |
| write_path | string | no | null | How created: session_summary, explicit_store, annotation, import |
| skip_redaction | bool | no | false | When true, bypasses secret/PII redaction |

**Response (200):**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "content": "User prefers dark mode in all editors",
  "type_hint": "preference",
  "source": "api",
  "tags": ["ui", "preference"],
  "created_at": "2026-03-10T12:00:00Z",
  "actor": null,
  "actor_type": "agent",
  "audience": "global",
  "embedding_status": "pending"
}
```

When `wait: true` and embedding completes, `embedding_status` will be `"complete"` instead of `"pending"`.

When content is redacted, includes:
```json
{
  "redactions": {"count": 1, "categories": ["aws_key"]}
}
```

**Errors:**

| Code | Condition |
|-|-|
| 400 | Empty content, resource cap exceeded, redaction error |
| 503 | Daemon not ready or store unavailable |
| 500 | Internal store failure |

**Example:**
```bash
curl -X POST http://localhost:9090/v1/store \
  -H "Content-Type: application/json" \
  -d '{
    "content": "User prefers dark mode in all editors",
    "type_hint": "preference",
    "tags": ["ui", "preference"],
    "source": "claude-agent"
  }'
```

---

### POST /v1/search

Hybrid search (BM25 + vector + symbolic) with salience re-ranking. Degrades to text-only when embedding provider is unavailable.

**Request Body:**

| Field | Type | Required | Default | Description |
|-|-|-|-|-|
| query | string | yes | - | Search query |
| limit | integer | no | 10 | Max results (1-100) |
| tags | string[] | no | null | Filter: memories must have ALL specified tags |
| source | string[] | no | null | Filter by source(s) |
| type_hint | string | no | null | Filter by type_hint |
| audience | string | no | null | Filter by audience scope |
| project | string | no | null | Project scope (returns project + global memories) |
| min_salience | float | no | config default | Minimum salience score (0.0-1.0) |
| fields | string[] | no | null | Field projection: only return these fields per result |
| cursor | string | no | null | Pagination cursor from previous response's next_cursor |

**Response (200):**
```json
{
  "results": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "content": "User prefers dark mode in all editors",
      "type_hint": "preference",
      "source": "claude-agent",
      "tags": ["ui", "preference"],
      "created_at": "2026-03-10T12:00:00Z",
      "actor": null,
      "actor_type": "agent",
      "audience": "global",
      "salience_score": 0.85,
      "composite_score": 0.92,
      "rrf_score": 0.016,
      "match_source": "hybrid"
    }
  ],
  "next_cursor": null,
  "has_more": false,
  "total": 1
}
```

`composite_score` is a 0-1 blend of retrieval similarity (RRF) and memory importance (salience, trust).

**Errors:**

| Code | Condition |
|-|-|
| 400 | Empty query, invalid min_salience, invalid cursor |
| 503 | Daemon not ready or store unavailable |
| 500 | Search or embedding failure |

**Example:**
```bash
curl -X POST http://localhost:9090/v1/search \
  -H "Content-Type: application/json" \
  -d '{
    "query": "dark mode preferences",
    "limit": 5,
    "tags": ["preference"]
  }'
```

---

### POST /v1/recall

Context injection for agent sessions. Supports query-based recall (vector similarity) and queryless cold-start recall (salience-ranked, no embedding needed).

**Request Body:**

| Field | Type | Required | Default | Description |
|-|-|-|-|-|
| query | string | no | null | Query text. Omit for queryless cold-start recall |
| session_id | string | no | auto-generated | Session ID for dedup tracking |
| first | bool | no | false | Session-start mode: pins project-summary, adds preamble/datetime |
| reset | bool | no | false | Clear session recall history (e.g., after context compaction) |
| project | string | no | null | Project scope filter |
| limit | integer | no | config default | Override max_memories for this request |
| boost_tags | string[] | no | [] | Tag affinity boost. Prefix matching: "channel:" boosts all channel:* |

**Response (200):**
```json
{
  "session_id": "sess_abc123",
  "count": 3,
  "memories": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "content": "User prefers dark mode...",
      "relevance": 0.92,
      "truncated": false
    }
  ],
  "summary": {
    "id": "660e9500-f30c-52e5-b827-557766551111",
    "content": "Project summary content..."
  },
  "current_datetime": "2026-03-10T12:00:00Z",
  "preamble": "You have access to persistent memory via memcp..."
}
```

The `summary`, `current_datetime`, and `preamble` fields are only present when `first: true`.

Memories may include `truncated: true` when content exceeds the configured truncation limit. Use the memory ID to fetch full content.

When `boost_tags` match, memories include `boost_applied: true` and `boost_score`.

When related context is enabled, memories may include `related_count` and `hint` (a suggested search command).

**Errors:**

| Code | Condition |
|-|-|
| 503 | Daemon not ready, store unavailable, or embedding provider unavailable (query-based only) |
| 500 | Recall or embedding failure |

**Example (queryless cold-start):**
```bash
curl -X POST http://localhost:9090/v1/recall \
  -H "Content-Type: application/json" \
  -d '{"first": true, "project": "memcp"}'
```

**Example (query-based):**
```bash
curl -X POST http://localhost:9090/v1/recall \
  -H "Content-Type: application/json" \
  -d '{
    "query": "what are the user UI preferences?",
    "session_id": "sess_abc123"
  }'
```

---

### POST /v1/annotate

Modify tags and/or salience on an existing memory. Returns a diff of changes.

**Request Body:**

| Field | Type | Required | Default | Description |
|-|-|-|-|-|
| id | string | yes | - | Memory ID |
| tags | string[] | no | null | Tags to append (merged with existing, deduplicated) |
| replace_tags | string[] | no | null | Tags to replace ALL existing tags (overrides `tags` if both given) |
| salience | string | no | null | Absolute ("0.9") or multiplier ("1.5x") for stability |

**Response (200):**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "changes": {
    "tags_added": ["important"],
    "tags_removed": [],
    "salience_before": 2.5,
    "salience_after": 3.75
  }
}
```

Salience fields only appear when salience was modified.

**Errors:**

| Code | Condition |
|-|-|
| 400 | Empty ID |
| 404 | Memory not found |
| 503 | Daemon not ready or store unavailable |
| 500 | Annotate failure |

**Example:**
```bash
curl -X POST http://localhost:9090/v1/annotate \
  -H "Content-Type: application/json" \
  -d '{
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "tags": ["important", "decision"],
    "salience": "1.5x"
  }'
```

---

### POST /v1/update

Replace memory content or metadata in place. Content changes trigger re-embedding.

**Request Body:**

| Field | Type | Required | Default | Description |
|-|-|-|-|-|
| id | string | yes | - | Memory ID |
| content | string | no | null | New content (triggers re-embedding) |
| type_hint | string | no | null | New type_hint |
| source | string | no | null | New source |
| tags | string[] | no | null | New tags (replaces all existing) |
| trust_level | float | no | null | Trust level override 0.0-1.0 (JSONB audit trail) |

At least one field besides `id` is required.

**Response (200):**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "content": "Updated preference: user prefers dark mode everywhere",
  "type_hint": "preference",
  "source": "claude-agent",
  "tags": ["ui", "preference"],
  "created_at": "2026-03-10T12:00:00Z",
  "actor": null,
  "actor_type": "agent",
  "audience": "global",
  "embedding_status": "pending"
}
```

When content changes, `embedding_status` resets to `"pending"` for re-embedding.

**Errors:**

| Code | Condition |
|-|-|
| 400 | Empty ID or no fields to update |
| 404 | Memory not found |
| 503 | Daemon not ready or store unavailable |
| 500 | Update failure |

**Example:**
```bash
curl -X POST http://localhost:9090/v1/update \
  -H "Content-Type: application/json" \
  -d '{
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "content": "Updated: user prefers dark mode with high contrast",
    "tags": ["ui", "preference", "accessibility"]
  }'
```

---

### DELETE /v1/memories/{id}

Hard-delete a memory by ID. The memory is permanently removed from PostgreSQL.

**Path Parameters:**

| Parameter | Type | Required | Description |
|-|-|-|-|
| id | string | yes | Memory UUID |

**Response (204):** No content.

**Errors:**

| Code | Condition |
|-|-|
| 400 | Empty ID |
| 404 | Memory not found or already deleted |
| 503 | Daemon not ready or store unavailable |
| 500 | Delete failure |

**Example:**
```bash
curl -X DELETE http://localhost:9090/v1/memories/550e8400-e29b-41d4-a716-446655440000
```

---

### GET /v1/status

Operational status with component health and resource usage. Alias for `/status`.

**Response (200):**
```json
{
  "status": "ok",
  "uptime_seconds": 3600,
  "components": {
    "database": "ok",
    "hnsw_index": "ok"
  },
  "resources": {
    "memory_count": 1500,
    "pending_embeddings": 0,
    "db_connections": 5,
    "pool_active": 2,
    "pool_idle": 3
  }
}
```

**Example:**
```bash
curl http://localhost:9090/v1/status
```

---

### GET /v1/export

Export memories in the requested format.

**Query Parameters:**

| Parameter | Type | Required | Default | Description |
|-|-|-|-|-|
| format | string | no | "jsonl" | Export format: jsonl, csv, markdown |
| project | string | no | null | Filter by project scope |
| tag | string (repeatable) | no | null | Filter: memories must have ALL specified tags |
| since | string | no | null | ISO 8601 timestamp -- only memories created after this |
| include_embeddings | string | no | "false" | "true" to include embedding vectors |
| include_state | string | no | "false" | "true" to include FSRS salience state |

**Response (200):**

Content-Type varies by format:
- `jsonl`: `application/x-ndjson`
- `csv`: `text/csv`
- `markdown`: `text/markdown`

**Errors:**

| Code | Condition |
|-|-|
| 400 | Invalid format or invalid since timestamp |
| 503 | Store unavailable |
| 500 | Export failure |

**Example:**
```bash
curl "http://localhost:9090/v1/export?format=jsonl&project=memcp&since=2026-03-01T00:00:00Z"
```

---

### POST /v1/discover

Cosine sweet-spot discovery for creative association. Finds memories in the similarity middle ground -- related enough to be meaningful but different enough to be surprising.

**Request Body:**

| Field | Type | Required | Default | Description |
|-|-|-|-|-|
| query | string | yes | - | Topic or concept to explore connections for |
| min_similarity | float | no | 0.3 | Lower bound (0.0-1.0). Lower = more surprising |
| max_similarity | float | no | 0.7 | Upper bound (0.0-1.0). Higher = more obviously related |
| limit | integer | no | 10 | Max results (1-50) |
| project | string | no | null | Project scope filter |

**Response (200):**
```json
{
  "discoveries": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "content": "Memory about design patterns...",
      "type_hint": "fact",
      "tags": ["architecture"],
      "similarity": "0.452",
      "created_at": "2026-03-08T10:30:00Z",
      "project": "memcp"
    }
  ],
  "query": "code organization",
  "similarity_range": [0.3, 0.7],
  "count": 1
}
```

**Errors:**

| Code | Condition |
|-|-|
| 400 | Empty query, min_similarity >= max_similarity |
| 503 | Daemon not ready, store unavailable, or embedding provider unavailable |
| 500 | Embedding or discovery failure |

**Example:**
```bash
curl -X POST http://localhost:9090/v1/discover \
  -H "Content-Type: application/json" \
  -d '{
    "query": "code organization",
    "min_similarity": 0.25,
    "max_similarity": 0.65,
    "limit": 5
  }'
```

---

## Non-versioned Endpoints

### GET /health

Liveness/readiness probe. Returns quickly (sub-ms AtomicBool check).

**Response (200):**
```json
{"status": "ok"}
```

**Response (503):** Daemon still starting.
```json
{"status": "starting"}
```

**Example:**
```bash
curl http://localhost:9090/health
```

---

### GET /metrics

Prometheus scrape endpoint. Returns metrics in Prometheus text exposition format.

**Response (200):** `text/plain` with Prometheus metrics.

Includes histograms for:
- `memcp_search_results_returned` -- results per search request
- `memcp_recall_memories_returned` -- memories per recall request
- `memcp_discover_results_returned` -- results per discover request

**Example:**
```bash
curl http://localhost:9090/metrics
```
