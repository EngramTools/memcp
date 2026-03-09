# memcp

**Persistent memory server for AI agents via MCP**

[![Version](https://img.shields.io/badge/version-0.1.0-blue)](https://github.com/SebaSeaBeVibing/memcp)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/built%20with-Rust-orange)](https://www.rust-lang.org)

---

## What is memcp?

memcp gives AI agents persistent, searchable memory. It stores knowledge across sessions using PostgreSQL with pgvector, and exposes it through the [Model Context Protocol (MCP)](https://modelcontextprotocol.io/), a direct HTTP API, or a CLI.

Agents store facts, decisions, and context during a session. On the next session they search for relevant memories and pick up exactly where they left off — no re-explaining, no lost state.

---

## Features

- **Semantic search** — fastembed (local CPU inference) or OpenAI embeddings
- **Hybrid search** — BM25 full-text + vector similarity + FSRS salience scoring
- **Query intelligence** — query expansion, result re-ranking, temporal hint parsing ("last week")
- **Background pipelines** — entity extraction, memory consolidation, semantic deduplication, GC
- **Trust-weighted retrieval** — provenance tracking with per-source trust multipliers
- **Content filtering** — regex patterns + semantic topic exclusion
- **Import** — Claude Code sessions, ChatGPT exports, OpenClaw logs, Markdown files
- **Discovery** — creative association discovery across unrelated memory clusters
- **CLI + daemon mode** — background workers, status monitoring, all storage operations from the terminal

---

## Quick Start

### Prerequisites

- Rust stable toolchain ([rustup.rs](https://rustup.rs))
- Docker (for PostgreSQL) or PostgreSQL 15+ with the [pgvector](https://github.com/pgvector/pgvector) extension

### Build

```bash
git clone https://github.com/SebaSeaBeVibing/memcp.git
cd memcp
cargo build --release
# Binary at: target/release/memcp
```

### Start PostgreSQL

Using Docker (recommended for local development):

```bash
docker run -d \
  --name memcp-postgres \
  -e POSTGRES_USER=memcp \
  -e POSTGRES_PASSWORD=memcp \
  -e POSTGRES_DB=memcp \
  -p 5433:5432 \
  ankane/pgvector:latest
```

Or if you have a `justfile`:

```bash
just pg
```

### Run the MCP Server

```bash
export DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp
cargo run --bin memcp -- serve
```

Migrations run automatically on first startup. The server listens on stdio for MCP connections.

---

## Claude Code MCP Configuration

Add memcp to your Claude Code MCP configuration (typically `~/.claude/claude_desktop_config.json` or `~/.config/claude/config.json`):

```json
{
  "mcpServers": {
    "memcp": {
      "command": "/path/to/memcp",
      "args": ["serve"]
    }
  }
}
```

Replace `/path/to/memcp` with the actual binary path (e.g., `target/release/memcp` relative to the repo, or a globally installed path).

After restarting Claude Code, the agent will have access to `store_memory`, `search_memory`, `recall_memory`, and other tools.

---

## CLI Usage

```bash
# Store a memory with tags
memcp store "Decided to use PostgreSQL with pgvector for the vector store" --tags architecture,decision

# Search memories by semantic similarity
memcp search "database architecture decisions"

# Recall memories for a project session (no query required)
memcp recall --first --project myproject

# List recent memories
memcp list --limit 20

# Export memories to JSON
memcp export --format json

# Start background workers (embedding, GC, extraction, consolidation)
memcp daemon

# Check daemon and system status
memcp status
```

---

## HTTP API

memcp exposes a REST API on the daemon's health port (default `:8080`). Key endpoints:

| Method | Path | Description |
|-|-|-|
| GET | `/health` | Liveness probe |
| GET | `/status` | Component health, pool stats, embedding info |
| GET | `/metrics` | Prometheus metrics |
| POST | `/v1/memories` | Store a memory |
| GET | `/v1/memories/search` | Semantic search |
| GET | `/v1/memories` | List memories with filters |
| DELETE | `/v1/memories/:id` | Delete a memory |

See `cargo doc --no-deps` for full API documentation.

---

## Configuration

memcp is configured via `memcp.toml` in the working directory, with environment variable overrides using `MEMCP__` prefix (double underscore for nesting):

```toml
database_url = "postgres://memcp:memcp@localhost:5433/memcp"
log_level = "info"

[embedding]
provider = "local"          # "local" (fastembed) or "openai"
model = "all-minilm-l6-v2"

[search]
bm25_backend = "native"     # "native" (tsvector) or "paradedb"
default_min_salience = 0.0

[gc]
enabled = true
salience_threshold = 0.1

[summarization]
enabled = false             # Requires Ollama or OpenAI
provider = "openai"
```

Example environment variable override:

```bash
MEMCP__DATABASE_URL=postgres://... MEMCP__EMBEDDING__PROVIDER=openai cargo run -- serve
```

For the full configuration reference and production deployment guide, see [docs/deployment.md](docs/deployment.md).

---

## Architecture

memcp is organized into four domain layers:

- **storage/** — PostgreSQL persistence, hybrid search (BM25 + pgvector), FSRS salience
- **intelligence/** — embedding providers, salience scoring, recall engine, query intelligence
- **pipeline/** — background workers (GC, dedup, extraction, consolidation, summarization, auto-store, enrichment)
- **transport/** — MCP server (rmcp), HTTP API (Axum), IPC, daemon orchestrator

See [ARCHITECTURE.md](ARCHITECTURE.md) for full module breakdown and data flow diagrams.

---

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code style, and workflow.

---

## License

MIT — see [LICENSE](LICENSE).
