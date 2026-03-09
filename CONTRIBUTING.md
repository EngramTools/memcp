# Contributing to memcp

We welcome contributions from the community! memcp is MIT licensed — fork, fix, and submit a pull request.

## Getting Started

1. Fork the repository on GitHub
2. Clone your fork: `git clone https://github.com/YOUR_USERNAME/memcp.git`
3. Create a feature branch: `git checkout -b feature/your-feature`
4. Make your changes following the development guidelines below
5. Run tests and lints (see commands below)
6. Commit with a conventional commit message
7. Push and open a pull request against `main`

## Prerequisites

- **Rust stable** (latest stable toolchain via `rustup`)
- **Docker** — for running PostgreSQL locally
- **PostgreSQL 15+** with the [pgvector](https://github.com/pgvector/pgvector) extension
- **just** (optional) — task runner for common commands (`cargo install just`)

## Running Locally

Start PostgreSQL with pgvector:

```bash
just pg
# Or manually:
docker run -d \
  --name memcp-postgres \
  -e POSTGRES_USER=memcp \
  -e POSTGRES_PASSWORD=memcp \
  -e POSTGRES_DB=memcp \
  -p 5433:5432 \
  ankane/pgvector:latest
```

Set the database URL:

```bash
export DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp
```

Start the MCP server (runs migrations automatically on startup):

```bash
cargo run --bin memcp -- serve
```

Run background workers (daemon mode):

```bash
cargo run --bin memcp -- daemon
```

## Development Commands

```bash
cargo build              # Build all crates
cargo test               # Run all tests
cargo clippy             # Lint (must produce zero warnings)
cargo fmt                # Format code
cargo doc --no-deps      # Generate API docs
```

Running tests requires a live database:

```bash
DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp cargo test -- --test-threads=1
```

## Code Style

- **Zero clippy warnings** — the CI build fails on any clippy warning. Run `cargo clippy` before pushing.
- **Formatted code** — run `cargo fmt` before committing. CI enforces `cargo fmt --check`.
- **Conventional commits** — commit messages follow the format `type(scope): description` (e.g., `feat(search): add salience filter`, `fix(storage): handle null embeddings`).

Common types: `feat`, `fix`, `test`, `refactor`, `chore`, `docs`.

## Adding a New Feature

1. Start with a failing test that captures the intended behavior
2. Implement the feature in the relevant module under `crates/memcp-core/src/`
3. Wire it into the CLI, MCP server, or HTTP API as appropriate
4. Add integration tests in `crates/memcp-core/tests/`
5. Update `ARCHITECTURE.md` if the feature adds a new module or changes system structure

## Project Structure

See [ARCHITECTURE.md](ARCHITECTURE.md) for a full overview of crate layout, domain layers, and data flow.

## Reporting Issues

Open a GitHub issue with:
- memcp version (`memcp --version`)
- Operating system and Rust toolchain version
- Steps to reproduce
- Expected vs. actual behavior

## License

memcp is licensed under the [MIT License](LICENSE). By contributing, you agree that your contributions will be licensed under the same terms.
