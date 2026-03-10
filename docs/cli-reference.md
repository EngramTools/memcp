# CLI Reference

All commands output JSON to stdout and errors to stderr with a non-zero exit code.

## Global Options

Every subcommand accepts these options:

| Option | Description |
|-|-|
| `--skip-migrate` | Skip automatic database migration on startup |
| `--remote <URL>` | Route commands through HTTP API instead of direct Postgres. Also settable via `MEMCP_URL` env var |
| `-h, --help` | Print help |
| `-V, --version` | Print version (top-level only) |

---

## Core Operations

### `memcp store`

Store a new memory.

**Synopsis:** `memcp store [OPTIONS] [CONTENT]`

| Option | Short | Default | Description |
|-|-|-|-|
| `[CONTENT]` | -- | -- | Memory content (positional). Omit if using --stdin |
| `--stdin` | -- | -- | Read content from stdin (for multi-paragraph content) |
| `--type-hint <TYPE>` | -- | `fact` | Memory type: fact, preference, instruction, decision, observation, summary |
| `--source <SOURCE>` | -- | `cli` | Source system identifier |
| `--tags <TAGS>` | -- | -- | Comma-separated tags |
| `--actor <ACTOR>` | -- | -- | Actor/agent name |
| `--actor-type <TYPE>` | -- | `agent` | Actor type |
| `--audience <AUD>` | -- | `global` | Audience scope |
| `--idempotency-key <KEY>` | -- | -- | At-most-once store semantics. Repeated calls with same key return original |
| `--wait` | -- | -- | Block until embedding completes (or timeout) |
| `--project <PROJECT>` | -- | -- | Project scope. Overrides MEMCP_PROJECT env var and config |
| `--trust-level <LEVEL>` | -- | -- | Trust level 0.0-1.0. Omit to let memcp infer |
| `--session-id <ID>` | -- | -- | Session identifier for grouping memories |
| `--agent-role <ROLE>` | -- | -- | Agent's role (e.g., coder, reviewer, planner) |
| `--no-redact` | -- | -- | Bypass secret/PII redaction |

**Examples:**

```bash
# Store a simple fact
memcp store "PostgreSQL uses MVCC for concurrency control" --type-hint fact --tags postgres,database

# Store a decision with project scope
memcp store "We chose JWT over session cookies for API auth" --type-hint decision --project myapp

# Store multi-paragraph content from stdin
echo "Long content here..." | memcp store --stdin --type-hint architecture

# Store with idempotency (safe to retry)
memcp store "API key rotation policy: 90 days" --idempotency-key "policy-rotation-v1"
```

---

### `memcp search`

Search memories by keyword + metadata matching with salience ranking.

**Synopsis:** `memcp search [OPTIONS] <QUERY>`

| Option | Short | Default | Description |
|-|-|-|-|
| `<QUERY>` | -- | (required) | Search query text |
| `--limit <N>` | -- | `20` | Maximum results |
| `--created-after <TS>` | -- | -- | Filter: created after timestamp |
| `--created-before <TS>` | -- | -- | Filter: created before timestamp |
| `--tags <TAGS>` | -- | -- | Filter by tags (comma-separated) |
| `--source <SOURCE>` | -- | -- | Filter by source |
| `--audience <AUD>` | -- | -- | Filter by audience |
| `--type-hint <TYPE>` | -- | -- | Filter by memory type |
| `--verbose` | -- | -- | Include full metadata in output |
| `--json` | -- | -- | Output raw JSON matching MCP serve envelope |
| `--compact` | -- | -- | One line per result: id_short score snippet [tags] |
| `--cursor <CURSOR>` | -- | -- | Pagination cursor from previous search |
| `--fields <FIELDS>` | -- | -- | Field projection (comma-separated: content,tags,id) |
| `--min-salience <F>` | -- | -- | Minimum salience threshold (0.0-1.0) |
| `--project <PROJECT>` | -- | -- | Project scope (includes global memories) |

**Examples:**

```bash
# Basic search
memcp search "database connection pooling"

# Search with filters
memcp search "auth" --type-hint decision --tags security --limit 5

# Compact output for scripting
memcp search "deployment" --compact

# Paginated search
memcp search "rust patterns" --cursor "eyJvZmZzZXQiOjIwfQ=="
```

---

### `memcp recall`

Recall relevant memories for automatic context injection at session start.

**Synopsis:** `memcp recall [OPTIONS] [QUERY]`

| Option | Short | Default | Description |
|-|-|-|-|
| `[QUERY]` | -- | -- | Query text (omit for query-less cold start) |
| `--session-id <ID>` | -- | auto-generated | Session ID |
| `--reset` | -- | -- | Clear session recall history before recalling (context compaction) |
| `--project <PROJECT>` | -- | -- | Project scope |
| `--first` | -- | -- | Session start mode -- injects datetime, preamble, and memories |
| `--limit <N>` | -- | config (3) | Override max_memories config |
| `--boost-tags <TAGS>` | -- | -- | Comma-separated boost tags for tag-affinity ranking. Prefix matching supported |

**Examples:**

```bash
# Session start with preamble
memcp recall --first --project myapp

# Query-specific recall
memcp recall "how does the auth system work" --project myapp

# With tag boosting
memcp recall --first --boost-tags "channel:devops,agent:reviewer"

# Reset session and recall fresh
memcp recall --first --reset --session-id "session-123"
```

---

### `memcp list`

List memories with optional filters and pagination.

**Synopsis:** `memcp list [OPTIONS]`

| Option | Short | Default | Description |
|-|-|-|-|
| `--type-hint <TYPE>` | -- | -- | Filter by memory type |
| `--source <SOURCE>` | -- | -- | Filter by source |
| `--created-after <TS>` | -- | -- | Filter: created after timestamp |
| `--created-before <TS>` | -- | -- | Filter: created before timestamp |
| `--updated-after <TS>` | -- | -- | Filter: updated after timestamp |
| `--updated-before <TS>` | -- | -- | Filter: updated before timestamp |
| `--limit <N>` | -- | `20` | Maximum results |
| `--cursor <CURSOR>` | -- | -- | Pagination cursor |
| `--actor <ACTOR>` | -- | -- | Filter by actor |
| `--audience <AUD>` | -- | -- | Filter by audience |
| `--verbose` | -- | -- | Include full metadata |
| `--project <PROJECT>` | -- | -- | Project scope |

**Examples:**

```bash
# List recent decisions
memcp list --type-hint decision --limit 10

# List memories from a specific source
memcp list --source openclaw --created-after 2024-01-01T00:00:00Z
```

---

### `memcp get`

Retrieve a memory by ID.

**Synopsis:** `memcp get <ID>`

| Option | Short | Default | Description |
|-|-|-|-|
| `<ID>` | -- | (required) | Memory UUID |

**Examples:**

```bash
memcp get 550e8400-e29b-41d4-a716-446655440000
```

---

### `memcp delete`

Delete a memory by ID (permanent).

**Synopsis:** `memcp delete <ID>`

| Option | Short | Default | Description |
|-|-|-|-|
| `<ID>` | -- | (required) | Memory UUID |

**Examples:**

```bash
memcp delete 550e8400-e29b-41d4-a716-446655440000
```

---

### `memcp recent`

Show recent memories for session handoff with a configurable time window.

**Synopsis:** `memcp recent [OPTIONS]`

| Option | Short | Default | Description |
|-|-|-|-|
| `--since <WINDOW>` | -- | `30m` | Time window (e.g., "30m", "1h", "2h", "1d") |
| `--source <SOURCE>` | -- | -- | Filter by source system |
| `--actor <ACTOR>` | -- | -- | Filter by actor/agent name |
| `--limit <N>` | -- | `10` | Max results |
| `--verbose` | -- | -- | Include full metadata |

**Examples:**

```bash
# Last 30 minutes (default)
memcp recent

# Last 2 hours from a specific agent
memcp recent --since 2h --actor vita
```

---

## Memory Management

### `memcp reinforce`

Reinforce a memory to boost its salience in future searches.

**Synopsis:** `memcp reinforce <ID> [OPTIONS]`

| Option | Short | Default | Description |
|-|-|-|-|
| `<ID>` | -- | (required) | Memory UUID |
| `--rating <RATING>` | -- | `good` | Rating: `good` or `easy` |

**Examples:**

```bash
memcp reinforce 550e8400-e29b-41d4-a716-446655440000
memcp reinforce 550e8400-e29b-41d4-a716-446655440000 --rating easy
```

---

### `memcp feedback`

Provide relevance feedback for a memory (useful or irrelevant).

**Synopsis:** `memcp feedback <ID> <SIGNAL>`

| Option | Short | Default | Description |
|-|-|-|-|
| `<ID>` | -- | (required) | Memory UUID |
| `<SIGNAL>` | -- | (required) | Feedback signal: `useful` or `irrelevant` |

**Examples:**

```bash
memcp feedback 550e8400-e29b-41d4-a716-446655440000 useful
memcp feedback 550e8400-e29b-41d4-a716-446655440000 irrelevant
```

---

### `memcp annotate`

Annotate an existing memory -- add/replace tags and adjust salience.

**Synopsis:** `memcp annotate --id <ID> [OPTIONS]`

| Option | Short | Default | Description |
|-|-|-|-|
| `--id <ID>` | -- | (required) | Memory UUID |
| `--tags <TAGS>` | -- | -- | Tags to append (comma-separated, combined with existing) |
| `--replace-tags <TAGS>` | -- | -- | Replace all tags (overrides --tags if both given) |
| `--salience <VALUE>` | -- | -- | Absolute (e.g., "0.9") or multiplier (e.g., "1.5x") |

**Examples:**

```bash
# Add tags
memcp annotate --id 550e8400-... --tags important,reviewed

# Replace all tags
memcp annotate --id 550e8400-... --replace-tags architecture,v2

# Boost salience by 50%
memcp annotate --id 550e8400-... --salience 1.5x
```

---

### `memcp update`

Update a memory's content or metadata in place.

**Synopsis:** `memcp update <ID> [CONTENT] [OPTIONS]`

| Option | Short | Default | Description |
|-|-|-|-|
| `<ID>` | -- | (required) | Memory UUID |
| `[CONTENT]` | -- | -- | New content (positional). Omit if using --stdin |
| `--stdin` | -- | -- | Read content from stdin |
| `--type-hint <TYPE>` | -- | -- | New type hint (replaces existing) |
| `--source <SOURCE>` | -- | -- | New source (replaces existing) |
| `--tags <TAGS>` | -- | -- | New tags (replaces existing, comma-separated) |
| `--wait` | -- | -- | Block until re-embedding completes |

**Examples:**

```bash
# Update content
memcp update 550e8400-... "Updated project summary with Q2 changes"

# Update metadata only
memcp update 550e8400-... --type-hint decision --tags architecture,final

# Update from stdin with wait
echo "New long-form content..." | memcp update 550e8400-... --stdin --wait
```

---

## Discovery

### `memcp discover`

Discover unexpected memory connections via cosine sweet-spot search. Finds memories that are related-but-different (0.3-0.7 similarity) for creative exploration and lateral thinking.

**Synopsis:** `memcp discover <QUERY> [OPTIONS]`

| Option | Short | Default | Description |
|-|-|-|-|
| `<QUERY>` | -- | (required) | Topic or concept to explore |
| `--min-similarity <F>` | -- | `0.3` | Minimum cosine similarity (lower = more surprising) |
| `--max-similarity <F>` | -- | `0.7` | Maximum cosine similarity (higher = more obvious) |
| `--limit <N>` | `-l` | `10` | Maximum results |
| `--project <PROJECT>` | -- | -- | Project scope filter |
| `--json` | -- | -- | Output raw JSON |

**Examples:**

```bash
# Explore connections to "authentication"
memcp discover "authentication"

# Wider similarity range for more creative results
memcp discover "database optimization" --min-similarity 0.2 --max-similarity 0.8

# JSON output for scripting
memcp discover "rust patterns" --json --limit 5
```

---

## Import / Export

### `memcp import`

Import memories from external sources.

**Synopsis:** `memcp import <SOURCE> [OPTIONS]`

**Sources:**

| Subcommand | Description |
|-|-|
| `jsonl <PATH>` | Import from JSONL file (round-trip with `memcp export`) |
| `openclaw [PATH]` | Import from OpenClaw SQLite (auto-detected if omitted) |
| `claude-code [PATH]` | Import from Claude Code MEMORY.md files (auto-detected if omitted) |
| `chatgpt <PATH>` | Import from ChatGPT export ZIP |
| `claude <PATH>` | Import from Claude.ai export ZIP |
| `markdown <PATH>` | Import from Markdown files or directory |
| `discover` | Auto-detect importable sources on this machine |
| `review` | Review what was filtered in the last import |
| `rescue [ID]` | Rescue filtered items from a previous import |

#### Common Import Options

All import sources (except discover, review, rescue) share these options:

| Option | Short | Default | Description |
|-|-|-|-|
| `--dry-run` | -- | -- | Preview without writing to database |
| `--project <PROJECT>` | -- | -- | Scope imported memories to this project |
| `--tags <TAGS>` | -- | -- | Extra tags for all imported memories (comma-separated) |
| `--skip-embeddings` | -- | -- | Skip embedding generation (status=pending) |
| `--batch-size <N>` | -- | `100` | Memories per database transaction |
| `--since <TS>` | -- | -- | Only import memories created after timestamp (ISO 8601) |
| `--skip-pattern <PAT>` | -- | -- | Additional noise patterns to filter (comma-separated) |
| `--no-filter` | -- | -- | Disable Tier 1 noise filtering |

#### Source-Specific Options

| Source | Option | Description |
|-|-|-|
| `openclaw` | `--agent <NAME>` | Filter to a specific agent name |
| `claude-code` | `--include-history` | Also import assistant messages from history.jsonl |
| `chatgpt` | `--curate` | LLM-assisted curation (summarize instead of chunk) |
| `claude` | `--curate` | LLM-assisted curation |
| `markdown` | `--curate` | LLM-assisted curation (classify chunks as keep/skip/merge) |

**Examples:**

```bash
# Import from Claude Code (auto-detect)
memcp import claude-code --project myapp

# Import from ChatGPT with curation
memcp import chatgpt ~/Downloads/chatgpt-export.zip --curate --project personal

# Dry run to preview
memcp import openclaw --dry-run

# Auto-detect sources
memcp import discover

# Review and rescue filtered items
memcp import review --last
memcp import rescue --all
```

#### `memcp import discover`

| Option | Short | Default | Description |
|-|-|-|-|
| `--yes` | -- | -- | Auto-accept all discovered sources without prompting |

#### `memcp import review`

| Option | Short | Default | Description |
|-|-|-|-|
| `--last` | -- | -- | Show results from the most recent import run |

#### `memcp import rescue`

| Option | Short | Default | Description |
|-|-|-|-|
| `[ID]` | -- | -- | Specific filtered item ID to rescue |
| `--all` | -- | -- | Rescue all filtered items from the last import |

---

### `memcp export`

Export memories to file.

**Synopsis:** `memcp export [OPTIONS]`

| Option | Short | Default | Description |
|-|-|-|-|
| `--format <FMT>` | -- | `jsonl` | Output format: `jsonl`, `csv`, or `markdown` |
| `--output <PATH>` | -- | stdout | Output file path |
| `--project <PROJECT>` | -- | -- | Filter by project |
| `--tags <TAGS>` | -- | -- | Filter by tags (comma-separated, must have ALL) |
| `--since <TS>` | -- | -- | Only export memories created on or after timestamp (ISO 8601) |
| `--include-embeddings` | -- | -- | Include embedding vectors in JSONL output |
| `--include-state` | -- | -- | Include FSRS/salience state in output |

**Examples:**

```bash
# Export all memories to JSONL
memcp export --output backup.jsonl

# Export project memories as markdown
memcp export --format markdown --project myapp --output memories.md

# Export with embeddings for migration
memcp export --format jsonl --include-embeddings --output full-backup.jsonl
```

---

## Daemon and Server

### `memcp daemon`

Start background workers (embedding, extraction, consolidation, auto-store, GC, curation, enrichment).

**Synopsis:** `memcp daemon [OPTIONS] [COMMAND]`

| Subcommand | Description |
|-|-|
| `install` | Install daemon as a system service (launchd on macOS, systemd on Linux) |

Running `memcp daemon` without a subcommand starts the daemon in the foreground.

**Examples:**

```bash
# Start daemon (foreground)
memcp daemon

# Install as system service
memcp daemon install
```

---

### `memcp serve`

Start MCP server on stdio (backwards-compatible mode for MCP clients).

**Synopsis:** `memcp serve [OPTIONS]`

No additional options beyond global options.

**Examples:**

```bash
# Start MCP server (typically called by MCP client config)
memcp serve
```

---

### `memcp status`

Show daemon status and pending work counts.

**Synopsis:** `memcp status [OPTIONS]`

| Option | Short | Default | Description |
|-|-|-|-|
| `--pretty` | -- | -- | Human-readable one-liner output |
| `--check` | -- | -- | Deep health check (pings DB, Ollama, checks model cache, watch paths) |

**Examples:**

```bash
# Quick status
memcp status

# Human-readable
memcp status --pretty

# Deep health check
memcp status --check
```

---

## Maintenance

### `memcp gc`

Run or preview garbage collection (prune low-salience and expired memories).

**Synopsis:** `memcp gc [OPTIONS]`

| Option | Short | Default | Description |
|-|-|-|-|
| `--dry-run` | -- | -- | Show candidates without making changes |
| `--salience-threshold <F>` | -- | from config | Override salience threshold |
| `--min-age-days <N>` | -- | from config | Override minimum age in days |

**Examples:**

```bash
# Preview what would be pruned
memcp gc --dry-run

# Run GC with custom threshold
memcp gc --salience-threshold 0.5 --min-age-days 14
```

---

### `memcp curation`

AI brain curation -- merge related memories, flag stale ones, strengthen important ones.

**Synopsis:** `memcp curation <COMMAND> [OPTIONS]`

| Subcommand | Description |
|-|-|
| `run` | Run a single curation pass |
| `log` | Show curation run history |
| `undo <RUN_ID>` | Undo a curation run |

#### `memcp curation run`

| Option | Short | Default | Description |
|-|-|-|-|
| `--propose` | -- | -- | Preview actions without executing (dry run) |

#### `memcp curation log`

| Option | Short | Default | Description |
|-|-|-|-|
| `--limit <N>` | -- | `10` | Number of recent runs to show |
| `--run-id <ID>` | -- | -- | Show detailed actions for a specific run |

#### `memcp curation undo`

| Option | Short | Default | Description |
|-|-|-|-|
| `<RUN_ID>` | -- | (required) | The run ID to undo |

**Examples:**

```bash
# Preview curation actions
memcp curation run --propose

# Run curation
memcp curation run

# View history
memcp curation log --limit 5

# View specific run details
memcp curation log --run-id 42

# Undo a run
memcp curation undo 42
```

---

### `memcp migrate`

Run database migrations and exit.

**Synopsis:** `memcp migrate [OPTIONS]`

No additional options beyond global options.

**Examples:**

```bash
memcp migrate
```

---

## Embedding Management

### `memcp embed`

Embedding management operations.

**Synopsis:** `memcp embed <COMMAND> [OPTIONS]`

| Subcommand | Description |
|-|-|
| `backfill` | Queue all un-embedded or failed memories for re-embedding |
| `stats` | Show embedding statistics (counts by model, pending, failed) |
| `switch-model` | Switch to a new embedding model |

#### `memcp embed backfill`

No additional options beyond global options.

```bash
memcp embed backfill
```

#### `memcp embed stats`

No additional options beyond global options.

```bash
memcp embed stats
```

#### `memcp embed switch-model`

| Option | Short | Default | Description |
|-|-|-|-|
| `--model <MODEL>` | -- | (required) | New model name (e.g., "text-embedding-3-small", "BGEBaseENV15") |
| `--dry-run` | -- | -- | Show what would happen without making changes |
| `--yes` | `-y` | -- | Skip confirmation prompt for destructive cross-dimension switches |

**Examples:**

```bash
# Preview model switch
memcp embed switch-model --model BGEBaseENV15 --dry-run

# Switch model (will prompt for confirmation if dimensions differ)
memcp embed switch-model --model text-embedding-3-small

# Non-interactive switch
memcp embed switch-model --model BGEBaseENV15 --yes
```

---

## Status Line

### `memcp statusline`

Manage Claude Code status line integration.

**Synopsis:** `memcp statusline <COMMAND> [OPTIONS]`

| Subcommand | Description |
|-|-|
| `install` | Install status line script to `~/.claude/scripts/` |
| `remove` | Remove status line script |

**Examples:**

```bash
memcp statusline install
memcp statusline remove
```
