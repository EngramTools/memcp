# Deployment Guide

This guide covers deploying memcp from source for local development and self-hosted production use.

---

## Prerequisites

- **Rust stable** — install via [rustup.rs](https://rustup.rs)
- **PostgreSQL 15+** with the [pgvector](https://github.com/pgvector/pgvector) extension
- **Docker** (optional, but recommended for local Postgres setup)

---

## Database Setup

### Option A: Docker (recommended for development)

```bash
docker run -d \
  --name memcp-postgres \
  -e POSTGRES_USER=memcp \
  -e POSTGRES_PASSWORD=memcp \
  -e POSTGRES_DB=memcp \
  -p 5433:5432 \
  ankane/pgvector:latest
```

The `ankane/pgvector` image includes pgvector pre-installed.

Connection URL: `postgres://memcp:memcp@localhost:5433/memcp`

### Option B: Manual PostgreSQL Setup

1. Install PostgreSQL 15+ and the pgvector extension:

   ```bash
   # Ubuntu/Debian
   sudo apt install postgresql-15 postgresql-15-pgvector

   # macOS (Homebrew)
   brew install postgresql pgvector
   ```

2. Create the database and user:

   ```sql
   CREATE USER memcp WITH PASSWORD 'memcp';
   CREATE DATABASE memcp OWNER memcp;
   \c memcp
   CREATE EXTENSION vector;
   ```

---

## Building

```bash
git clone https://github.com/SebaSeaBeVibing/memcp.git
cd memcp
cargo build --release
```

The binary is at `target/release/memcp`. You can copy it to your `$PATH`:

```bash
sudo cp target/release/memcp /usr/local/bin/memcp
```

---

## Configuration

memcp uses layered configuration: built-in defaults, then `memcp.toml` in the working directory, then environment variables (prefix `MEMCP__` with double underscores for nesting).

### Example `memcp.toml`

```toml
database_url = "postgres://memcp:memcp@localhost:5433/memcp"
log_level = "info"

[embedding]
# "local" uses fastembed (CPU inference, no API key needed)
# "openai" uses OpenAI embeddings API
provider = "local"
model = "all-minilm-l6-v2"

[search]
# "native" uses PostgreSQL tsvector (no extra extensions needed)
# "paradedb" uses ParadeDB pg_search extension (better BM25)
bm25_backend = "native"
default_min_salience = 0.0

[salience]
w_recency = 0.25
w_access = 0.15
w_semantic = 0.45
w_reinforce = 0.15
recency_lambda = 0.01    # ~70-day half-life

[extraction]
enabled = true
provider = "ollama"       # "ollama" or "openai"
ollama_base_url = "http://localhost:11434"
ollama_model = "llama3.2:3b"
# openai_api_key = "sk-..."   # only needed when provider = "openai"

[consolidation]
enabled = true
similarity_threshold = 0.90

[gc]
enabled = true
salience_threshold = 0.05
min_age_days = 30
ttl_days = 365
purge_grace_days = 7

[dedup]
enabled = true
similarity_threshold = 0.95

[summarization]
enabled = false
provider = "openai"
# openai_api_key = "sk-..."

[query_intelligence]
# enabled = false by default — requires LLM provider
# expansion_provider = "ollama"
# reranking_provider = "ollama"

[content_filter]
enabled = false
# excluded_topics = ["harmful content", "private data"]

[recall]
max_memories = 20
min_relevance = 0.0
session_ttl_seconds = 3600

[health]
port = 8080
bind = "127.0.0.1"

[resource_caps]
max_memories = 100000
max_search_results = 100
```

### Environment Variable Overrides

All config keys can be overridden with environment variables using the `MEMCP__` prefix and double underscores for nesting:

```bash
export MEMCP__DATABASE_URL=postgres://user:pass@host/db
export MEMCP__LOG_LEVEL=debug
export MEMCP__EMBEDDING__PROVIDER=openai
export MEMCP__EMBEDDING__OPENAI_API_KEY=sk-...
export MEMCP__SEARCH__BM25_BACKEND=paradedb
export MEMCP__GC__ENABLED=false
```

---

## Running

### MCP Server Mode (for Claude Code integration)

```bash
export DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp
memcp serve
```

Migrations run automatically on startup. The server listens on stdio for MCP protocol messages.

### Daemon Mode (background workers)

```bash
memcp daemon
```

Spawns all background workers:
- Embedding pipeline (async batch embedding)
- Extraction pipeline (entity/fact extraction via LLM)
- Consolidation worker (merge near-duplicate memories)
- Dedup worker (semantic deduplication after embedding)
- GC worker (salience-threshold garbage collection)
- Auto-store worker (JSONL session file ingestion)
- Enrichment worker (retroactive neighbor enrichment)
- IPC listener (for CLI commands that need embedding/reranking)
- Health server (HTTP on the configured port)

### CLI Mode

```bash
# Store a memory
memcp store "Content here" --tags tag1,tag2 --project myproject

# Search memories
memcp search "query" --project myproject --limit 10

# List recent memories
memcp list --limit 20

# Recall (returns top memories by salience, no query needed)
memcp recall --first --project myproject

# Export all memories
memcp export --format json > memories.json

# Import from a file
memcp import --format json memories.json

# Check daemon status
memcp status
```

---

## Embedding Providers

### fastembed (local, default)

- Model: `all-minilm-l6-v2` (384 dimensions)
- No API key required
- Runs on CPU — model downloads automatically on first use (~100MB)
- Suitable for most deployments

```toml
[embedding]
provider = "local"
model = "all-minilm-l6-v2"
```

### OpenAI

- Model: `text-embedding-3-small` (1536 dimensions) or `text-embedding-ada-002`
- Requires `OPENAI_API_KEY` environment variable or config key
- Better semantic quality for English text; API cost per embedding

```toml
[embedding]
provider = "openai"
model = "text-embedding-3-small"
openai_api_key = "sk-..."   # or use MEMCP__EMBEDDING__OPENAI_API_KEY
```

---

## Migrations

Migrations run automatically when the server starts. They are located in `crates/memcp-core/migrations/` and managed by sqlx (compile-time checked).

To run migrations manually:

```bash
memcp migrate
```

To check migration status:

```bash
DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp sqlx migrate info \
  --source crates/memcp-core/migrations
```

---

## Production Considerations

### Connection Pool

The default connection pool size is 5. For production workloads, increase it via:

```toml
# memcp.toml
[database]
max_connections = 20
```

Or use `DATABASE_POOL_MAX_SIZE` environment variable.

### pgvector Index Tuning

memcp creates an HNSW index on memory embeddings. For large datasets (>100k memories), tune the index parameters:

```sql
-- After bulk import, rebuild the index with higher m and ef_construction:
DROP INDEX IF EXISTS memory_embeddings_embedding_idx;
CREATE INDEX memory_embeddings_embedding_idx ON memory_embeddings
  USING hnsw (embedding vector_cosine_ops)
  WITH (m = 32, ef_construction = 200);
```

### Log Levels

Set `MEMCP__LOG_LEVEL` to control verbosity:

- `error` — production default (failures only)
- `warn` — recommended for production (includes deprecation notices)
- `info` — default (startup, shutdown, worker activity)
- `debug` — verbose, includes SQL queries and embedding stats

### Health Monitoring

The daemon exposes health endpoints:

```bash
# Liveness check
curl http://localhost:8080/health

# Component status (pool, workers, embedding provider)
curl http://localhost:8080/status

# Prometheus metrics
curl http://localhost:8080/metrics
```

Integrate `/health` with your load balancer or container orchestrator for automatic restarts.

### GC Configuration

By default, GC removes memories with salience below 0.05 that are older than 30 days. Tune for your workload:

```toml
[gc]
enabled = true
salience_threshold = 0.1     # More aggressive pruning
min_age_days = 14             # Prune after 2 weeks instead of 30 days
ttl_days = 180                # Hard TTL: 6 months
purge_grace_days = 7          # Time between soft-delete and hard purge
```
