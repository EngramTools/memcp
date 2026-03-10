# memcp

**Persistent memory server for AI agents via MCP**

[![Version](https://img.shields.io/badge/version-0.1.0-blue)](https://github.com/SebaSeaBeVibing/memcp)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/built%20with-Rust-orange)](https://www.rust-lang.org)

## What is memcp?

memcp gives AI agents persistent, searchable memory. It stores knowledge across sessions using PostgreSQL with pgvector and exposes it through the [Model Context Protocol (MCP)](https://modelcontextprotocol.io/), an HTTP API, or a CLI. Agents store facts, decisions, and context during a session. On the next session they search for relevant memories and pick up where they left off.

## Install

**From source:**

```bash
git clone https://github.com/SebaSeaBeVibing/memcp.git
cd memcp
cargo build --release
# Binary at: target/release/memcp
```

**Docker (PostgreSQL):**

```bash
docker run -d --name memcp-postgres \
  -e POSTGRES_USER=memcp -e POSTGRES_PASSWORD=memcp -e POSTGRES_DB=memcp \
  -p 5433:5432 ankane/pgvector:latest
```

## Quickstart

```bash
# Start PostgreSQL (if using justfile)
just pg

# Start the MCP server (migrations run automatically)
export DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp
cargo run --bin memcp -- serve

# Store a memory
memcp store "Use PostgreSQL with pgvector for the vector store" --tags architecture,decision

# Search by semantic similarity
memcp search "database architecture decisions"

# Recall top memories for a project (no query needed)
memcp recall --first --project myproject

# Start background workers (embedding, GC, extraction, consolidation)
memcp daemon
```

## MCP Configuration

Add memcp to your Claude Code MCP config (`~/.claude.json` or project `.mcp.json`):

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

After restarting, the agent has access to `store_memory`, `search_memory`, `recall_memory`, and other tools.

## Documentation

| Guide | Description |
|-|-|
| [Architecture](docs/architecture.md) | Module structure, data flow, design decisions |
| [Deployment](docs/deployment.md) | Production deployment and container setup |
| [Configuration](memcp.toml.example) | Annotated example config with all options |

## Features

- **Hybrid search** -- BM25 full-text + pgvector similarity + FSRS salience scoring
- **Query intelligence** -- query expansion, re-ranking, temporal hint parsing, multi-query decomposition
- **Background pipelines** -- entity extraction, consolidation, semantic dedup, GC, enrichment, curation
- **Trust-weighted retrieval** -- provenance tracking with per-source trust multipliers
- **Content filtering** -- regex patterns + semantic topic exclusion
- **Redaction** -- automatic secret detection and PII masking on ingestion
- **Import** -- Claude Code sessions, ChatGPT exports, OpenClaw logs, Markdown files
- **Auto-store sidecar** -- watches conversation logs and ingests memories automatically
- **CLI + daemon mode** -- background workers, status monitoring, all operations from the terminal

## engram.host

[engram.host](https://engram.host) is the managed hosting service for memcp. It handles PostgreSQL, embeddings, and background workers so you do not have to. memcp is the open-source core; engram.host adds multi-tenant isolation and hosted infrastructure.

## License

MIT -- see [LICENSE](LICENSE).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code style, and workflow.
