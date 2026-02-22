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
