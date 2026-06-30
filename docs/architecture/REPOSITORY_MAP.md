# forgecode — Repository Map

## Top-Level Layout

```
forgecode/
├── Cargo.toml           # Workspace root (32–34 member crates)
├── Cargo.lock
├── crates/              # All workspace crates (see below)
├── src/                 # TypeScript entry (vestigial, see SSOT: N/A)
│   ├── index.ts
│   ├── domain/
│   ├── ports/
│   ├── adapters/
│   └── app/
├── docs/                # Documentation
│   ├── tool-guidelines.md
│   ├── architecture/    # This directory
│   │   ├── ARCHITECTURE.md
│   │   └── REPOSITORY_MAP.md
│   ├── adr/
│   │   └── 0001-compaction-summarization-strategy.md
│   ├── fork-sync/
│   │   └── upstream-audit-20260629.md
│   ├── journeys/
│   │   └── manifests/README.md
│   ├── operations/
│   │   ├── iconography/SPEC.md
│   │   └── journey-traceability.md
│   ├── sessions/
│   │   └── 20260628-forgecode-overhaul/
│   └── tasks/
│       └── task-compaction-enhancement.md
├── commands/            # Built-in slash command markdown files
│   └── github-pr-description.md
├── templates/           # Handlebars prompt templates
│   ├── forge-system-prompt-title-generation.md
│   ├── forge-summarization-prompt.md
│   ├── forge-enhanced-summary-frame.md
│   ├── forge-partial-*.md
│   └── ...
├── .forge/commands/     # User-defined custom slash commands
│   ├── check.md
│   ├── fixme.md
│   …                     # (review, test, think, exec, pipe, capture, parent, subagents)
├── scripts/             # Tooling scripts
├── tooling/             # Tooling configurations
├── tests/               # Integration/E2E tests
├── benches/             # Criterion benchmarks
├── shell-plugin/        # ZSH plugin
│   └── forge.plugin.zsh
├── AGENTS.md            # Agent guidelines (this repo)
├── CLAUDE.md            # Claude-specific guidelines
├── CONTRIBUTING.md      # Contribution guide
├── Justfile             # Build task runner
├── flake.nix            # Nix flake
└── README.md
```

## Crate Map (34 crates)

### Domain & Application

| Crate                  | Path                          | Purpose                                              |
| ---------------------- | ----------------------------- | ---------------------------------------------------- |
| `forge_domain`         | `crates/forge_domain/`        | Pure domain: models, traits, errors, tool DSL, compact |
| `forge_app`            | `crates/forge_app/`           | Orchestrator, tool registry, hooks, retry, pipeline  |
| `forge_services`       | `crates/forge_services/`      | Provider auth, conversations, commands, MCP, policy  |
| `forge_api`            | `crates/forge_api/`           | Public API surface, re-exports domain + app          |
| `forge_config`         | `crates/forge_config/`        | Retry config, filesystem layout, tunables            |

### CLI & UI

| Crate                    | Path                              | Purpose                                      |
| ------------------------ | --------------------------------- | -------------------------------------------- |
| `forge_main`             | `crates/forge_main/`              | Binary: CLI, TUI, zsh plugin, stream renderer |
| `forge_display`          | `crates/forge_display/`           | Diff format, syntax highlight, grep, markdown |
| `forge_select`           | `crates/forge_select/`            | Interactive fuzzy picker (nucleo-based)       |
| `forge_spinner`          | `crates/forge_spinner/`           | Terminal spinner/progress indicators          |
| `forge_snaps`            | `crates/forge_snaps/`             | Snapshot testing infrastructure (insta)       |
| `forge_template`         | `crates/forge_template/`          | Handlebars template engine, XML element builder |
| `forge_tui`              | `crates/forge_tui/`               | Ratatui-based terminal UI components          |

### Streaming & Events

| Crate                      | Path                               | Purpose                                    |
| -------------------------- | ---------------------------------- | ------------------------------------------ |
| `forge_stream`             | `crates/forge_stream/`             | MpscStream channel-based async stream       |
| `forge_eventsource`        | `crates/forge_eventsource/`        | SSE client with reconnection + retry       |
| `forge_eventsource_stream` | `crates/forge_eventsource_stream/` | Tokio-stream wrapper for EventSource       |
| `forge_markdown_stream`    | `crates/forge_markdown_stream/`    | Streaming markdown → terminal renderer     |

### Infrastructure

| Crate              | Path                           | Purpose                                 |
| ------------------ | ------------------------------ | --------------------------------------- |
| `forge_infra`      | `crates/forge_infra/`          | Env, FS, process, HTTP, auth, MCP client |
| `forge_fs`         | `crates/forge_fs/`             | Filesystem operations                    |
| `forge_repo`       | `crates/forge_repo/`           | Provider repos (OpenAI, Anthropic, etc.) |
| `forge_embed`      | `crates/forge_embed/`          | Workspace embedding / semantic search    |

### Persistence

| Crate          | Path                         | Purpose                                           |
| -------------- | ---------------------------- | ------------------------------------------------- |
| `forge_dbd`    | `crates/forge_dbd/`          | WIP: SQLite daemon for persistent conversation storage |

### Similarity & Drift Detection

| Crate               | Path                            | Purpose                                      |
| ------------------- | ------------------------------- | -------------------------------------------- |
| `forge_similarity`  | `crates/forge_similarity/`      | Similarity scoring: hash-only, local ONNX, hosted fallback |
| `forge_drift`       | `crates/forge_drift/`           | Drift detection: hash + Jaccard word-set for multi-agent overlap |

### CI

| Crate       | Path                      | Purpose                              |
| ----------- | ------------------------- | ------------------------------------ |
| `forge_ci`  | `crates/forge_ci/`        | CI workflow generation (gh-workflow)  |

### 3D / Graphviz

| Crate     | Path                      | Purpose                             |
| --------- | ------------------------- | ----------------------------------- |
| `forge3d` | `crates/forge3d/`         | 3D visualisation / Graphviz output  |

### Terminal Multiplexer

| Crate        | Path                        | Purpose                              |
| ------------ | --------------------------- | ------------------------------------ |
| `forge_mux`  | `crates/forge_mux/`         | Terminal multiplexer bridge (tmux)   |

### Utilities

| Crate                | Path                             | Purpose                              |
| -------------------- | -------------------------------- | ------------------------------------ |
| `forge_json_repair`  | `crates/forge_json_repair/`      | JSON repair / fix malformed JSON     |
| `forge_tool_macros`  | `crates/forge_tool_macros/`      | Procedural macros for ToolDescription derive |
| `forge_walker`       | `crates/forge_walker/`           | Directory walker / file tree traversal |
| `forge_tracker`      | `crates/forge_tracker/`          | Session metrics and telemetry        |

### Testing

| Crate              | Path                           | Purpose                              |
| ------------------ | ------------------------------ | ------------------------------------ |
| `forge_test_kit`   | `crates/forge_test_kit/`       | Test infrastructure, fakes, fixtures |

### Shell & Terminal

| Crate                      | Path                               | Purpose                              |
| -------------------------- | ---------------------------------- | ------------------------------------ |
| `forge_pheno_shell`        | `crates/forge_pheno_shell/`        | Phenotype shell integration          |
| `forge_pheno_winterminal`  | `crates/forge_pheno_winterminal/`  | WinterMinal terminal bridge          |
| `ghostty-kit`              | `crates/ghostty-kit/`              | Ghostty terminal kit support         |

## Key File Paths

### Slash Commands

| File                                     | Description                          |
| ---------------------------------------- | ------------------------------------ |
| `commands/github-pr-description.md`      | Built-in PR description command      |
| `.forge/commands/`                       | User-defined custom command dir      |
| `crates/forge_domain/src/command.rs`     | Command model (name + description + prompt) |
| `crates/forge_services/src/command.rs`   | Command loader/discovery service     |
| `crates/forge_app/src/command_generator.rs` | Command generation/prompt rendering |

### CLI

| File                                             | Description                    |
| ------------------------------------------------ | ------------------------------ |
| `crates/forge_main/src/main.rs`                  | Entry point                    |
| `crates/forge_main/src/cli.rs`                   | CLI argument definitions       |
| `crates/forge_main/src/terminal/mod.rs`          | Terminal/REPL loop             |
| `crates/forge_main/src/zsh/plugin.rs`            | ZSH plugin code gen            |
| `crates/forge_main/src/state.rs`                 | Conversation state management  |

### Core Pipeline

| File                                          | Description                         |
| --------------------------------------------- | ----------------------------------- |
| `crates/forge_app/src/app.rs`                 | ForgeApp::chat() entry             |
| `crates/forge_app/src/orch.rs`                | Orchestrator loop                  |
| `crates/forge_app/src/tool_registry.rs`       | Tool resolution + dispatch         |
| `crates/forge_app/src/tool_executor.rs`       | System tool execution              |
| `crates/forge_app/src/agent_executor.rs`      | Agent delegation executor          |
| `crates/forge_app/src/mcp_executor.rs`        | MCP tool execution                 |
| `crates/forge_app/src/system_prompt.rs`       | System prompt assembly             |
| `crates/forge_app/src/user_prompt.rs`         | User prompt generation             |
| `crates/forge_app/src/hooks/mod.rs`           | Hook chain definitions             |
| `crates/forge_app/src/retry.rs`               | HTTP retry with backon             |

### Models & DTOs

| File                                                  | Description                |
| ----------------------------------------------------- | -------------------------- |
| `crates/forge_app/src/dto/openai/`                    | OpenAI DTOs + transformers |
| `crates/forge_app/src/dto/anthropic/`                 | Anthropic DTOs + transforms |
| `crates/forge_app/src/dto/google/`                    | Google Gemini DTOs         |

### Domain

| File                                           | Description                       |
| ---------------------------------------------- | --------------------------------- |
| `crates/forge_domain/src/tools/catalog.rs`     | ToolCatalog enum (all built-in tools) |
| `crates/forge_domain/src/tools/descriptions/`  | Tool description markdown files   |
| `crates/forge_domain/src/compact/`             | Compaction/summarization module   |
| `crates/forge_domain/src/policies/`            | Policy engine for restricted mode |
| `crates/forge_domain/src/error.rs`             | Error types (Retryable, etc.)     |
| `crates/forge_domain/src/command.rs`           | Command model (slash commands)    |

### Configuration

| File                                    | Description                   |
| --------------------------------------- | ----------------------------- |
| `crates/forge_config/src/config.rs`     | ForgeConfig struct            |
| `crates/forge_config/src/retry.rs`      | RetryConfig definition        |

### Resilience Contract

| File                                                            | Description                   |
| --------------------------------------------------------------- | ----------------------------- |
| `docs/contracts/provider-models/resilience-policy.schema.json`  | Cross-repo retry contract     |

## Agent Guidelines (AGENTS.md)

See `AGENTS.md` at repo root for:
- Error management conventions (anyhow, thiserror)
- Testing patterns (fixture/actual/expected, pretty_assertions, insta)
- Domain type construction (derive_setters)
- Service implementation guidelines (clean architecture, single type parameter)
- Git/CI operations (Co-Authored-By, gh, cargo check)
