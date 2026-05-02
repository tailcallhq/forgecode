# forgecode — CLAUDE.md

> **Fork of [tailcallhq/forgecode](https://github.com/tailcallhq/forgecode).**
> Phenotype-org additions: `deny.toml` + `cargo-deny.yml` CI bootstrapped 2026-05-01.

---

This repo is a **fork** of the upstream [tailcallhq/forgecode](https://github.com/tailcallhq/forgecode)
project — an AI-enhanced terminal development environment with ZSH plugin support,
TUI, and multi-provider LLM integration.

Do not rewrite upstream content. Any changes to upstream-origin files must be
clearly annotated as Phenotype-org-specific additions.

## Project Overview

| Field | Value |
|-------|-------|
| Workspace | Multi-crate (21 internal crates under `crates/`) |
| Edition | 2024 |
| Rust version | 1.92 |
| License | MIT |
| Upstream | <https://github.com/tailcallhq/forgecode> |

## Phenotype-Org Additions

The following files are Phenotype-org-specific additions (not present in upstream):

- `deny.toml` — cargo-deny configuration
- `cargo-deny.yml` — GitHub Actions CI workflow for dependency auditing

All other files follow upstream conventions.

## Stack

| Layer | Technology |
|-------|------------|
| Runtime | tokio (full, rt-multi-thread, macros, sync, fs, process, signal) |
| HTTP client | reqwest (rustls, hickory-dns, http2) |
| Auth | aws-config, aws-sdk-bedrockruntime, google-cloud-auth |
| CLI | clap 4.6 + clap_complete |
| TUI | reedline 0.47, rustyline 18, termimad, console |
| Serialization | serde, serde_json, serde_yml, toml_edit |
| Diff/patch | dissimilar, similar, strip-ansi-escapes |
| Search | grep-searcher, fzf-wrapped, ignore |
| MCP | rmcp (client + SSE + subprocess + streamable-http transports) |
| Observability | tracing, tracing-subscriber, posthog-rs |
| Git | gix |
| Misc | anyhow, thiserror, uuid, chrono, url, is_ci |

## Key Commands

```bash
# Build (from repo root)
cargo build --release

# Test
cargo test --workspace

# Format
cargo fmt --check

# Lint
cargo clippy --workspace --all-targets -- -D warnings

# Full quality gate
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

## Crate Map

```
crates/
├── forge_main        # Binary entry point
├── forge_app        # Application layer
├── forge_domain     # Domain types & logic
├── forge_infra      # Infrastructure / adapters
├── forge_api        # API layer
├── forge_embed      # Embedded resources
├── forge_ci         # CI utilities
├── forge_display    # Display / TUI rendering
├── forge_fs         # Filesystem operations
├── forge_repo       # Git repository integration
├── forge_services   # Service layer
├── forge_snaps      # Snapshot testing (insta)
├── forge_spinner    # Spinner / progress UI
├── forge_stream     # Streaming utilities
├── forge_template  # Template rendering (handlebars)
├── forge_tool_macros # Proc-macro helpers
├── forge_tracker    # Telemetry / tracking
├── forge_walker     # Directory traversal
├── forge_json_repair # JSON repair
├── forge_select     # Interactive selection (fzf)
├── forge_test_kit   # Test utilities
├── forge_markdown_stream # Markdown streaming
├── forge_config     # Configuration handling
├── forge_eventsource # Event source
└── forge_eventsource_stream # Event source streaming
```

## Quality Gates

- `cargo fmt --check` — formatting must pass
- `cargo clippy --workspace --all-targets -- -D warnings` — zero lints allowed
- `cargo test --workspace` — all tests must pass
- `cargo deny check` — dependency audit (configured in `deny.toml`)
- Snapshot tests via `insta` — review snapshots with `cargo insta review`

## CI / GitHub Actions

- `cargo-deny.yml` runs `cargo deny check advisories licenses` on every PR
- `deny.toml` defines allowlist rules for crates and licenses
- Run `cargo deny check` locally before opening PRs

## Git Workflow

```
origin  = KooshaPari/forgecode     (Phenotype-org fork)
upstream = tailcallhq/forgecode     (canonical upstream)
```

Sync from upstream:
```bash
git fetch upstream
git checkout main
git merge upstream/main
git push origin main
```

## Security & Compliance

- `deny.toml` + `cargo-deny.yml` enforce dependency audit (advisories + licenses)
- `cargo deny check` must pass before merging
