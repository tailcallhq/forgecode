# forgecode — Architecture

forgecode is an AI-enhanced terminal development environment (agentic coding CLI/TUI).
It is a Rust Cargo workspace built on a hexagonal (ports-and-adapters) architecture,
forged from [tailcallhq/forgecode](https://github.com/tailcallhq/forgecode).

## Architectural Pattern

```
Domain (pure, framework-free) ← Application Services ← Infrastructure (adapters)
```

| Layer            | Crate(s)                          | Responsibility                               |
| ---------------- | --------------------------------- | -------------------------------------------- |
| Domain           | `forge_domain`                    | Models, traits/ports, error types, tool DSL  |
| Application      | `forge_app`                       | Orchestrator, tool registry, hooks, retry    |
| Service glue     | `forge_services`                  | Provider auth, conversation, commands, MCP   |
| Composition root | `forge_api`                       | Public `API` trait, re-exports domain + app  |
| CLI main         | `forge_main`                      | `forge` binary: CLI, TUI, zsh plugin         |
| Config           | `forge_config`                    | Retry config, filesystem layout, tunables    |
| Infrastructure   | `forge_infra`                     | Env, FS, process, HTTP, auth, MCP client     |
| Repositories     | `forge_repo`                      | Provider repos (OpenAI, Anthropic, …)        |
| Streaming        | `forge_stream`, `forge_eventsource`, `forge_eventsource_stream`, `forge_markdown_stream` | SSE/event-source, streaming markdown renderer |
| Rendering        | `forge_display`, `forge_select`, `forge_spinner`, `forge_snaps`, `forge_template` | Diff display, picker, spinners, snapshots, templates |
| Persistence      | `forge_dbd` (WIP), `forge_repo`   | SQLite daemon (WIP), session/workspace repos  |
| Similarity/Drift | `forge_similarity`, `forge_drift` | Similarity scoring (hash/ONNX/hosted), multi-agent drift detection |
| CI               | `forge_ci`                        | CI workflow generation (gh-workflow bindings) |
| Utilities        | `forge_fs`, `forge_json_repair`, `forge_embed`, `forge_tool_macros`, `forge_walker` | FS operations, JSON repair, embedding, tool proc macros, directory walks |
| 3D/Graphviz      | `forge3d`                         | 3D visualisation / Graphviz rendering        |
| Terminal Mux     | `forge_mux`                       | Terminal multiplexer bridge (tmux)           |
| Testing          | `forge_test_kit`                  | Test infrastructure, fixtures, fakes         |
| Tracking         | `forge_tracker`                   | Session metrics and telemetry                |
| TUI              | `forge_tui`                       | Ratatui-based terminal UI components          |
| Shell plugins    | `forge_pheno_shell`, `forge_pheno_winterminal` | Phenotype shell integration, WinterMinal bridge |
| Ghostty plug-in  | `ghostty-kit`                     | Ghostty terminal kit integration             |

### Workspace Overview (34 crates)

Live count: `ls crates/ | wc -l` (34 crates on disk, 25 in workspace members as of latest ref).

## Core Request Pipeline

```
User prompt (CLI or piped stdin)
  → ForgeApp::chat() [crates/forge_app/src/app.rs]
    → Conversation resolution + file discovery
    → Agent + provider resolution + credential refresh
    → Tool definitions (system tools + MCP + agent tools)
    → System prompt assembly (templates)
    → User prompt generation
    → Orchestrator::run() [crates/forge_app/src/orch.rs]
      ├── Hook chain: on_start → on_request → on_response → on_end
      ├── Provider request (OpenAI/Anthropic/Google DTOs)
      ├── Tool call execution (system / agent / MCP)
      ├── Compaction handler (token budget enforcement)
      ├── Doom-loop detection
      └── Conversation persistence
  → SSE/JSON response stream to client
```

## Resilience & Stability Patterns

### Retry Policy (cross-repo contract)

Defined in `docs/contracts/provider-models/resilience-policy.schema.json` and implemented
in `forge_config::RetryConfig` + `forge_app::retry::retry_with_config` (backon crate).

| Parameter           | Default | Description                                |
| ------------------- | ------- | ------------------------------------------ |
| `max_attempts`      | 8       | Total attempts (initial + 7 retries)       |
| `initial_backoff_ms`| 200     | Base delay before first retry              |
| `min_delay_ms`      | 1000    | Floor on computed backoff                  |
| `backoff_factor`    | 2.0     | Exponential multiplier per attempt          |
| `max_delay_secs`    | null    | Optional cap on backoff (no cap by default) |
| `jitter`            | true    | Uniform random jitter to avoid thundering-herd |
| `suppress_errors`   | false   | Suppress retry log lines for non-critical ops |

**Retryable HTTP codes**: 408, 429, 500, 502, 503, 504, 520, 522, 524, 529.
**Non-retryable**: 400, 401, 403, 404, 405, 409, 410, 413, 422, 451.
**Retryable network errors**: network_timeout, connection_reset, connection_refused,
dns_resolution_failure, tls_handshake_timeout, read_timeout.

### SSE Reconnect

Separate from HTTP retry (distinct state machine in `forge_eventsource/src/retry.rs`).
Triggered by connection drops, not by terminal-marker rules.

### Circuit Breaker (Tool Error Tracker)

`ToolErrorTracker` in orchestrator: tolerates up to `max_tool_failure_per_turn` (default 3)
errors per agent turn before propagating failure. Prevents cascading tool errors.

### Hook System

Chainable lifecycle hooks: `on_start`, `on_request`, `on_response`, `on_toolcall_start`,
`on_toolcall_end`, `on_end`. Built-in handlers:
- `CompactionHandler` — token budget compaction after each turn
- `DoomLoopDetector` — detects repetitive tool-call patterns
- `PendingTodosHandler` — optional verification of pending todos at session end
- `TitleGenerationHandler` — async title generation per conversation
- `TracingHandler` — instrumentation spans for each lifecycle event

### Doom-Loop Detection

`forge_app/src/hooks/doom_loop.rs`: detects when the agent repeats the same tool call
with identical arguments. Triggers a reminder prompt to break the cycle.

### Compaction Strategy

`forge_domain::compact/`: sliding-window summarization with adaptive eviction, importance
scoring, prefiltering, and streaming support. Configurable per-agent with
`CompactionConfig`.

## Tool System

### Tool Catalog (Built-in Tools)

Defined in `forge_domain::tools::catalog::ToolCatalog`:

| Tool           | Description                          |
| -------------- | ------------------------------------ |
| `Read`         | Read file contents                   |
| `Write`        | Write content to file                |
| `FsSearch`     | Regex file search (ripgrep)          |
| `SemSearch`    | Semantic search (workspace index)    |
| `Remove`       | Remove files                         |
| `Patch`        | Apply code patch                     |
| `MultiPatch`   | Apply multiple patches atomically    |
| `Undo`         | Undo last file operation             |
| `Shell`        | Execute shell command                |
| `Fetch`        | HTTP/HTTPS fetch                     |
| `Followup`     | Follow-up message to assistant       |
| `Plan`         | Create/modify implementation plan    |
| `Skill`        | Load and execute a skill             |
| `TodoWrite`    | Create/update todo items             |
| `TodoRead`     | Read current todo items              |
| `Task`         | Delegate subtask to another agent    |

### Tool Resolution

- `ToolRegistry` (forge_app) resolves tool calls to: system tools (ToolCatalog),
  agent-delegation tools, or MCP tools.
- Glob-based allowlisting per agent (e.g. `mcp_*`, `read?`, `[abc]`).
- Modality validation for image-enabled tools against the active model.
- Policy enforcement (restricted mode) via `forge_domain::policies/`.

### Custom Commands (Slash Commands)

YAML-frontmatter `.md` files in `.forge/commands/` (user-defined) or `commands/`
(built-in). Loaded by `CommandLoaderService` (forge_services) and made available
as slash commands in the UI. Resolution priority:

1. User-defined (`.forge/commands/`) overrides built-in
2. Built-in (`commands/`)

**Built-in commands**: `github-pr-description`

**User-defined command examples**: `review`, `test`, `think`, `exec`, `pipe`,
`capture`, `check`, `fixme`, `parent`, `subagents`

Each command file uses YAML frontmatter (`name`, `description`) with a Markdown
body as the prompt template. Parameters are appended after the command name
(e.g. `/review --focus=security`). The `Command` model is in
`forge_domain::command::Command`.

## Streaming

### Stream Types

| Stream              | Crate                  | Purpose                        |
| ------------------- | ---------------------- | ------------------------------ |
| `MpscStream`        | `forge_stream`         | Channel-based async stream     |
| `EventSource`       | `forge_eventsource`    | SSE client for provider streaming |
| `EventSourceStream` | `forge_eventsource_stream` | Tokio-stream wrapper      |
| `Streamdown`        | `forge_markdown_stream`| Streaming markdown → terminal renderer |

### SSE Reconnection

`forge_eventsource` implements SSE reconnection with exponential backoff + jitter,
separate from the HTTP-request retry loop. The `is_sse_terminal()` helper detects
provider-specific stream-end markers.

## Agent System

### Agent Types (built-in)

| Agent   | Role                                         |
| ------- | -------------------------------------------- |
| `forge` | Primary coding agent (default)               |
| `sage`  | Research agent (read, search, fetch only)    |
| `debug` | Debugging agent (shell + search + fetch)     |
| Custom  | User-defined via agent template in config    |

Agents are defined with: model, provider, tool allowlist, compaction config, max
tool failures, temperature, reasoning effort.

### Agent Delegation (Task Tool)

The `Task` tool delegates work to sub-agents. Configured via:
- `--agent` / `--aid` CLI flag
- `research_subagent` config toggle (enables/disables sage/agent)
- Per-agent tool allowlists

## Persistence

### SQLite Session Store

- WAL journaling mode with dedicated checkpointer
- zstd context compression for conversation history
- Incremental auto-vacuum
- FTS5 full-text search + vector search
- WIP: `forge_dbd` — standalone SQLite daemon for persistent conversation storage

### Workspace Index

`forge_workspace` (embedded via `forge_embed`): semantic search index backed by
SQLite FTS5 + vector embeddings using `fastembed` (local ONNX) or hosted provider.

### Conversation Model

Conversations store: messages, context (with token counts), metadata (title, agent,
model, provider), timestamps, parent/child relationships for subagent breadcrumbs.

## Observability

- Opentelemetry tracing via `tracing` crate + `tracing-subscriber` (json + env-filter)
- Session metrics in `forge_tracker` (token usage, tool call counts, duration)
- `forge info --porcelain` for machine-readable session info
- `forge logs` for log tailing

## License

MIT. See LICENSE and LICENSE-APACHE files.
