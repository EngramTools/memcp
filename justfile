# memcp development commands

# Default recipe: show available commands
default:
    @just --list

# Build the project
build:
    cargo build

# Build release binary
build-release:
    cargo build --release

# Run all tests (requires PostgreSQL running via `just pg`)
# sqlx::test uses DATABASE_URL as the base to create/drop ephemeral test databases.
# Your dev data in the memcp database is never touched by tests.
test:
    DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp cargo test

# Run unit tests only
test-unit:
    cargo test --test unit

# Run integration tests only
test-integration:
    DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp cargo test --test recall_test --test feedback_test --test embedding_pipeline_test --test gc_dedup_test --test summarization_test --test store_test

# Run E2E tests only
test-e2e:
    DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp cargo test --test journey_test --test auto_store_e2e --test mcp_contract

# Run all tests using cargo-nextest (faster parallel execution)
test-fast:
    DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp cargo nextest run --workspace

# Run golden search quality tests (requires fastembed model via local-embed feature)
test-golden:
    DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp cargo test --test search_quality -- --ignored --nocapture

# Run all tests with coverage report (opens HTML report in browser)
coverage:
    DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp cargo llvm-cov --all-features --workspace --html --open

# Run all tests with coverage and fail if below threshold (matches CI threshold)
coverage-check:
    DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp cargo llvm-cov --all-features --workspace --fail-under-lines 75

# Run clippy lints
lint:
    cargo clippy -- -D warnings

# Run rustfmt check
fmt-check:
    cargo fmt -- --check

# Format code
fmt:
    cargo fmt

# Run database migrations (requires DATABASE_URL)
migrate:
    cargo run -- migrate

# Start Docker Compose services (postgres + app)
up:
    docker compose up -d

# Start with rebuild
up-build:
    docker compose up -d --build

# Stop Docker Compose services
down:
    docker compose down

# Stop and remove volumes (clean slate)
down-clean:
    docker compose down -v

# View logs
logs:
    docker compose logs -f

# View app logs only
logs-app:
    docker compose logs -f app

# Start just PostgreSQL (for native development)
pg:
    docker compose up -d postgres

# Run all checks (lint + fmt + test)
check: lint fmt-check test

# Full CI simulation
ci: lint fmt-check test build-release
