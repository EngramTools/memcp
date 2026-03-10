# QA Playbook

Manual sanity check walkthrough for memcp. Follow step-by-step against a clean isolated database.

For automated agent-driven testing, see `qa/agent/` YAML test cases.

## Prerequisites

- Docker installed and running
- Rust toolchain (stable)
- memcp built: `cargo build --release`
- `psql` client available (for DB inspection)
- `jq` installed (optional, for JSON parsing)

## Setup: Isolated QA Database

**IMPORTANT:** QA uses its own Postgres on port 5434. Never test against your dev database (5433).

```bash
# Start isolated QA Postgres
docker run -d --name memcp_qa_postgres \
  -e POSTGRES_USER=memcp_qa \
  -e POSTGRES_PASSWORD=memcp_qa \
  -e POSTGRES_DB=memcp_qa \
  -p 5434:5432 \
  pgvector/pgvector:pg17

# Wait for it to be ready
sleep 3
pg_isready -h localhost -p 5434 -U memcp_qa

# Run migrations
DATABASE_URL=postgres://memcp_qa:memcp_qa@localhost:5434/memcp_qa \
  ./target/release/memcp migrate
```

Set the env var for all subsequent commands:

```bash
export DATABASE_URL=postgres://memcp_qa:memcp_qa@localhost:5434/memcp_qa
```

---

## Journey 1: Store -> Search -> Recall

The core loop: store a memory, find it via search, then recall it in a session.

**Step 1: Store a memory**

```bash
memcp store "PostgreSQL uses MVCC for concurrency control" \
  --type-hint fact --tags postgres,database --wait
```

Expected: `Stored memory <uuid>` message with a UUID.

**Step 2: Search for it**

```bash
memcp search "database concurrency" --json
```

Expected: JSON output containing the stored memory. Look for `"content"` field with "MVCC" in it.

**Step 3: Search with compact output**

```bash
memcp search "database concurrency" --compact
```

Expected: One-line-per-result format showing ID, score, snippet, and tags.

**Step 4: Recall at session start**

```bash
memcp recall --first
```

Expected: Output includes datetime preamble and any stored memories. The MVCC memory should appear.

**Step 5: Query-specific recall**

```bash
memcp recall "how does PostgreSQL handle concurrency"
```

Expected: Recall output includes the MVCC memory as relevant context.

---

## Journey 2: Feedback Loop

Store a memory, provide feedback, verify salience changes affect ranking.

**Step 1: Store two related memories**

```bash
memcp store "Redis supports pub/sub for real-time messaging" \
  --type-hint fact --tags redis --wait

memcp store "RabbitMQ uses AMQP protocol for message queuing" \
  --type-hint fact --tags rabbitmq --wait
```

**Step 2: Search and note the results**

```bash
memcp search "message queue systems" --json
```

Note which memory appears first and its score.

**Step 3: Mark one as useful**

Get the ID from the search results (the `"id"` field), then:

```bash
memcp feedback <MEMORY_ID> useful
```

Expected: Success message. The memory's salience is boosted.

**Step 4: Re-search and compare**

```bash
memcp search "message queue systems" --json
```

Expected: The reinforced memory should rank higher (or maintain position) compared to unreinforced ones.

**Step 5: Mark a memory as irrelevant**

```bash
memcp feedback <OTHER_MEMORY_ID> irrelevant
```

Expected: Success message. That memory's salience decreases.

---

## Journey 3: GC Flow

Store low-value memories, run garbage collection, verify pruning.

**Step 1: Store several low-value memories**

```bash
memcp store "Temporary note about lunch plans" --type-hint observation --wait
memcp store "TODO: check email later" --type-hint observation --wait
memcp store "Meeting at 3pm tomorrow" --type-hint observation --wait
```

**Step 2: Preview GC candidates (dry-run)**

```bash
memcp gc --dry-run
```

Expected: Output listing candidate memories for pruning. No deletions happen.

**Step 3: Run GC with aggressive threshold**

```bash
memcp gc --salience-threshold 0.99 --min-age-days 0
```

Expected: Memories below the salience threshold are pruned. Output shows count of pruned memories.

**Step 4: Verify pruned memories are gone**

```bash
memcp search "lunch plans" --json
```

Expected: The pruned memory no longer appears in results.

---

## Journey 4: Import / Export Round-trip

Export memories, verify file format, import back.

**Step 1: Store test data**

```bash
memcp store "Export test: architecture decision record" \
  --type-hint decision --tags export-test,architecture --wait
memcp store "Export test: deployment configuration" \
  --type-hint fact --tags export-test,devops --wait
```

**Step 2: Export as JSONL**

```bash
memcp export --format jsonl --tags export-test --output /tmp/qa_export.jsonl
```

Expected: File created at `/tmp/qa_export.jsonl`. Each line is a valid JSON object.

```bash
head -1 /tmp/qa_export.jsonl | jq .
```

Expected: JSON with `content`, `type_hint`, `tags`, `created_at` fields.

**Step 3: Export as CSV**

```bash
memcp export --format csv --tags export-test --output /tmp/qa_export.csv
```

Expected: CSV file with header row including `content`, `type_hint`, `tags`.

**Step 4: Import dry-run**

```bash
memcp import jsonl /tmp/qa_export.jsonl --dry-run
```

Expected: Preview of what would be imported, without writing to database.

**Step 5: Clean up**

```bash
rm -f /tmp/qa_export.jsonl /tmp/qa_export.csv
```

---

## Journey 5: Daemon Lifecycle

Start, check, and stop the daemon process.

**Step 1: Check status (no daemon running)**

```bash
memcp status --pretty
```

Expected: Status output showing database connection info and pending work counts.

**Step 2: Deep health check**

```bash
memcp status --check
```

Expected: Detailed health check output including DB connectivity, model cache, and worker status.

**Step 3: Embedding stats**

```bash
memcp embed stats
```

Expected: Counts of embedded, pending, and failed memories by model.

**Step 4: Recent memories**

```bash
memcp recent --since 1h
```

Expected: List of memories created in the last hour (should include QA test data).

---

## Teardown

After completing QA, clean up the isolated environment:

```bash
# Remove QA Postgres container
docker rm -f memcp_qa_postgres

# Clean up any temp files
rm -f /tmp/qa_*.jsonl /tmp/qa_*.csv /tmp/qa_*.md /tmp/qa_memory_id*.txt

# Unset env var
unset DATABASE_URL
```

---

## Quick Smoke Test

For a fast sanity check (under 2 minutes), run only these steps:

1. Setup (start Docker, run migrations)
2. `memcp store "smoke test" --type-hint fact --wait`
3. `memcp search "smoke test" --json` (verify result returned)
4. `memcp recall --first` (verify recall works)
5. `memcp list --limit 1` (verify list works)
6. `memcp status --pretty` (verify status works)
7. Teardown

If all six commands succeed with expected output, core functionality is working.
