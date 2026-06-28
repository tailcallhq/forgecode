# forgecode

An AI-enhanced terminal development environment — an agentic coding CLI/TUI with ZSH plugin support, built in Rust.

> **Fork of [tailcallhq/forgecode](https://github.com/tailcallhq/forgecode).** This fork (`forge-dev`) adds Phenotype-specific features (SQLite session store with WAL checkpointing + zstd compression, conversation FTS/vector search, subagent breadcrumbs) on top of upstream.

## Status

| Check | State |
|-------|-------|
| Default branch | `main` |
| Language | Rust (2021 edition) |
| Binary | `forge` (from `crates/forge_main`) |
| Version | 2.10.0 |
| License | MIT / Apache-2.0 |

## Architecture

A Cargo workspace of 33 crates following a hexagonal (ports-and-adapters) layout. The domain is pure and framework-free; infrastructure and providers are adapters behind traits, composed at the application root.

```
crates/
  forge_domain/      — pure domain: models, traits/ports, no I/O framework deps
  forge_app/         — composition root: wires services + adapters into the domain
  forge_services/    — orchestration / business logic over the domain
  forge_api/         — public API surface (the `API` async-trait boundary)
  forge_infra/       — infrastructure adapters (env, fs, process, http)
  forge_repo/        — persistence + provider repositories (OpenAI, Anthropic, …)
  forge_dbd/         — SQLite session daemon (WIP) over a Unix socket
  forge_main/        — the `forge` binary (CLI/TUI entrypoint)
  forge_stream/ forge_eventsource/ forge_markdown_stream/ — streaming/SSE
  forge_walker/ forge_fs/ forge_similarity/ forge_drift/ forge_json_repair/ — utilities
  forge_template/ forge_select/ forge_spinner/ forge_display/ forge_snaps/ — TUI/render
  forge_tracker/ forge_embed/ forge_config/ forge_mux/ forge_ci/ — cross-cutting
  forge3d/           — 3D/visualization server
  forge_pheno_shell/ forge_pheno_winterminal/ — shell/terminal integration
  forge_tool_macros/ forge_test_kit/ — tooling + test support
```

See `docs/SSOT.md` for the authoritative state-of-the-repo and `CLAUDE.md`/`AGENTS.md` for contributor governance.

## Quick Start

```sh
# Build the workspace
cargo build --release

# Run the CLI
cargo run --bin forge

# Tests (prefers cargo-nextest; falls back to cargo test)
cargo nextest run    # or: cargo test

# Lint + format
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Or via the `Justfile`:

```sh
just build    # cargo build
just test     # cargo nextest run (fallback cargo test)
just lint     # clippy -D warnings + fmt --check
just fmt      # cargo fmt
```

## Configuration & secrets

Credentials are stored locally at `~/.forge` / `.credentials.json` with `0o600` permissions and are gitignored. Never commit credentials; use environment variables or the local credential store.

## Contributing

Read `CLAUDE.md` and `AGENTS.md` first — they are the canonical contributor contract. CI gates on `cargo fmt --check`, `cargo clippy -D warnings`, and the test suite (Linux runner).
