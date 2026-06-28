# forgecode Justfile — Rust Cargo workspace
set shell := ["bash", "-cu"]

# Show available commands
default:
    @just --list

# Build the workspace
build:
    cargo build

# Build optimized release
release:
    cargo build --release

# Run the forge CLI
run *ARGS:
    cargo run --bin forge -- {{ARGS}}

# Run tests (prefer nextest, fall back to cargo test)
test:
    @if command -v cargo-nextest >/dev/null 2>&1; then cargo nextest run; else cargo test; fi

# Lint: clippy (deny warnings) + format check
lint:
    cargo clippy --all-targets --all-features -- -D warnings
    cargo fmt --all -- --check

# Auto-format code
fmt:
    cargo fmt --all

# CI-like run (build + test + lint)
ci: build test lint

# Clean build artifacts
clean:
    cargo clean
