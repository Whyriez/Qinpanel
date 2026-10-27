# Qinpanel Justfile
set shell := ["bash", "-uc"]

default:
    @just --list

# Run both panel-agent and panel-api for development
dev:
    @echo "==> Starting Qinpanel in development mode..."
    @echo "Note: panel-agent requires root on Linux. Run in dev VM for full functionality."
    cargo run -p panel-api

# Run all test suites across backend and frontend
test:
    @echo "==> Running Rust test suite..."
    cargo test --workspace
    @echo "==> Running Web frontend test suite..."
    cd web && npm test -- --run

# Run code formatters and linters
lint:
    @echo "==> Checking Rust formatting..."
    cargo fmt --check
    @echo "==> Running Clippy linters..."
    cargo clippy --workspace --all-targets -- -D warnings
    @echo "==> Running Web linter and type-checker..."
    cd web && npm run lint && npm run check

# Run integration & isolation test suite inside Dev VM
test-vm:
    @echo "==> Running test suite inside dev VM..."
    ./scripts/dev-vm.sh test

# Run cross-tenant isolation test suite
test-isolation:
    @echo "==> Running cross-tenant isolation test suite..."
    cargo test -p isolation-tests

# Build all binaries and frontend assets
build:
    @echo "==> Building Web assets..."
    cd web && npm run build
    @echo "==> Building Rust release binaries..."
    cargo build --workspace --release

# Format code
fmt:
    cargo fmt
    cd web && npm run format || true

# Run performance benchmark harness & validate PRD Section 9 non-functional targets
benchmark:
    @echo "==> Running benchmark validation against PRD Section 9 targets..."
    cargo run -p panel-cli -- benchmark validate-targets

# Run performance comparison suite (baseline vs tuned)
benchmark-compare:
    @echo "==> Running tuning profile comparison..."
    cargo run -p panel-cli -- benchmark compare --target php --baseline baseline --tuned opcache_enabled --format markdown
