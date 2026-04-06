default:
    @just --list

# Run all checks (what CI runs)
check: fmt-check clippy test

# Auto-format all code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all --check

# Run clippy lints
clippy:
    cargo clippy --all-targets --workspace -- -D warnings

# Run unit tests
test:
    cargo test --workspace

# Run integration tests (requires Ollama)
test-integration:
    cargo test --test integration -- --ignored

# Run all tests including integration
test-all: test test-integration

# Build in debug mode
build:
    cargo build --workspace

# Build in release mode
build-release:
    cargo build --release

# Run security audit on dependencies
audit:
    cargo audit

# Run cargo-deny checks (licenses, bans, advisories, sources)
deny:
    cargo deny check

# Build the sandbox Docker image
build-sandbox:
    docker build -t gclaw-sandbox:latest -f Dockerfile.sandbox .

# Run gclaw in TUI mode
run:
    cargo run --release

# Run gclaw in headless mode
run-headless:
    cargo run --release -- --headless

# Clean build artifacts
clean:
    cargo clean

# Generate and open documentation
doc:
    cargo doc --workspace --no-deps --open

# Set up the development environment
setup:
    @echo "Installing git hooks..."
    git config core.hooksPath .githooks
    @echo "Copying example config..."
    cp -n config.example.toml config.toml 2>/dev/null || true
    @echo "Done! Edit config.toml to set your provider and model."

# Tag a new release (usage: just release 0.2.0)
release version:
    git tag -a "v{{version}}" -m "Release v{{version}}"
    @echo "Tagged v{{version}}. Push with: git push origin v{{version}}"
