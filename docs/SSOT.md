# SSOT — forgecode

Authoritative state-of-the-repo. forgecode is an AI-enhanced terminal development environment (agentic coding CLI/TUI), a Rust Cargo workspace, fork of [tailcallhq/forgecode](https://github.com/tailcallhq/forgecode).

## State
- Default branch: main
- Last verified: 2026-06-28
- Binary: `forge` (crates/forge_main, v2.10.0)
- CI status: green

## Dependencies
- Rust: 2021 edition (Cargo workspace, 33 crates)
- Node: N/A (no JS/TS product code)
- Python: tooling-only (governance propagation scripts)

## Architecture
- Pattern: hexagonal (ports-and-adapters)
- Domain (pure, framework-free): `forge_domain` — models + traits/ports
- Composition root: `forge_app` wires `forge_services` + adapters into the domain
- Public API boundary: `forge_api` (the async-trait `API`)
- Adapters: `forge_infra` (env/fs/process/http), `forge_repo` (persistence + provider repos: OpenAI, Anthropic, …)
- Persistence: SQLite via Diesel + r2d2 pool (WAL mode, busy_timeout, dedicated checkpointer); `forge_dbd` session daemon (WIP)
- Streaming: `forge_stream`, `forge_eventsource`, `forge_markdown_stream`
- TUI/render: `forge_display`, `forge_select`, `forge_spinner`, `forge_snaps`, `forge_template`

## Fork-specific features (this fork vs upstream)
- SQLite session store: WAL checkpointing, zstd context compression, incremental auto_vacuum
- Conversation FTS5 + vector search; sort/filter wired into the TUI conv-view
- Subagent breadcrumbs ("spawned by X") in the info panel / conv header

## Next Steps (DAG)
See `docs/sessions/20260628-forgecode-overhaul/03_DAG_WBS.md` for the active phased overhaul roadmap (P0 de-fork docs → P1 CI gates/stubs/security → P2 resilience/observability/lifecycle → P3 perf/concurrency → P4 ops/threat-model → P5 cross-repo shared crates).

## Fleet Links
- Parent: Phenotype
- Upstream: tailcallhq/forgecode
- Related: OmniRoute, cliproxyapi-plusplus (shared provider/OAuth/resilience logic — P5 extraction candidates)
