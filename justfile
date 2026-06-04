# octo dev tasks — run `just <task>`. Install: cargo install just
set dotenv-load := true

# List available tasks.
default:
    @just --list

# Build the whole workspace.
build:
    cargo build --workspace

# Run all tests.
test:
    cargo test --workspace

# Format the code.
fmt:
    cargo fmt --all

# Check formatting (CI mode).
fmt-check:
    cargo fmt --all -- --check

# Lint with clippy, warnings as errors.
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Dependency/license/advisory audit.
deny:
    cargo deny check

# Everything CI runs.
ci: fmt-check lint test deny

# Start local Postgres.
db-up:
    docker compose up -d db

# Stop local services.
db-down:
    docker compose down

# Run database migrations.
migrate:
    sqlx migrate run --source crates/store/migrations

# Drop & recreate the dev database schema (destructive — local only).
db-reset:
    docker exec -i octo-db psql -U octo -d octo -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"

# Run the server.
run:
    cargo run -p octo-server
