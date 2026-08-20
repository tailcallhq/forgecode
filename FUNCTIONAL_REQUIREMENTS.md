# forgecode — Functional Requirements Catalog (FR-NNN)

Machine-readable acceptance contract for forgecode (published as `helioslite`).
Every FR records a **real** capability of this repository with acceptance
criteria and the test location that guards it. Status values:

- `implemented` — capability ships and is covered by automated tests in-repo.
- `partial` — capability ships but acceptance is manual or only partly covered.
- `planned` — capability is agreed, not yet implemented (tracked in issues).

New work must reference the FR it changes via `FR-NNN` in the PR description
(see `.github/PULL_REQUEST_TEMPLATE.md`). The catalog is the traceability
backbone for the [journey manifests](docs/journeys/README.md) and the
[tool-execution audit trail](docs/security/threat-model.md) gap register.

## Catalog

### FR-001 — Core agentic loop

- **Title:** Interactive and single-shot agent session loop.
- **Status:** implemented
- **Description:** `forge` starts an interactive read-eval-print loop when stdin
  is a TTY with no arguments; `forge -p "<prompt>"` runs one prompt and exits;
  piped stdin (`cat prompt.txt | forge`) runs the piped content as the first
  message. Sessions are multi-turn and keep conversation state across turns.
- **Acceptance criteria:**
  - `forge` with no args and a TTY stdin enters the interactive line editor
    and accepts input without error.
  - `forge -p "hello"` prints an agent response and exits with code 0.
  - `cat prompt.txt | forge` processes piped input as a droppable message and
    does not block waiting for a terminal.
  - Non-TTY stdin without `-p` never enters the interactive loop (CI-safe).
- **Test location:** `crates/forge_main/src/cli.rs:984` (`mod tests`),
  `crates/forge_main/src/main.rs:217` (`mod tests`).

### FR-002 — Tool execution

- **Title:** Agent tool registry, executor, and sandbox.
- **Status:** implemented
- **Description:** Tools are registered in `forge_app` (tool_registry.rs) and
  executed by tool_executor.rs. Subprocesses run with scrubbed environments
  (incl. `NO_COLOR` removal) inside an optional isolated git worktree
  (`--sandbox`) for experimentation.
- **Acceptance criteria:**
  - Each registered tool resolves by name to a typed executor.
  - Tool output is returned to the agent loop as a structured result.
  - `--sandbox <name>` creates an isolated git worktree before the session and
    subprocess env vars are scrubbed deterministically (no `NO_COLOR` leak).
  - Tool execution failures surface as typed errors, not panics.
- **Test location:** `crates/forge_app/src/tool_registry.rs`,
  `crates/forge_app/src/tool_executor.rs`, insta snapshots in
  `crates/forge_app/src/snapshots/` (63 `.snap` files), sandbox in
  `crates/forge_main/src/sandbox.rs`.

### FR-003 — TUI & keyboard interaction

- **Title:** Interactive line editor, fuzzy pickers, progress, and streaming render.
- **Status:** implemented
- **Description:** Sessions use a rustyline-based line editor; purpose-built
  fuzzy pickers (model/agent/provider/workspace) via `forge select`; themed
  progress via forge_spinner; incremental markdown streaming via
  forge_markdown_stream. Windows consoles use the Win32 path for correct
  color/spinner rendering.
- **Acceptance criteria:**
  - `forge select model -q <query>` prints `model_id` on line 1 and
    `provider_id` on line 2; prints nothing on cancel.
  - Multi-line and history navigation work in the line editor.
  - Streaming output renders incrementally and completes to full output.
  - Keyboard-driven workflows (tab completion, arrow navigation, escape to
    cancel) work in the pickers.
- **Test location:** `crates/forge_select/src/{input,multi,preview,select}.rs`
  (`mod tests`), `crates/forge_spinner/src/{lib,progress_bar}.rs` (`mod tests`),
  render snapshots in `crates/forge_app/src/snapshots/`.

### FR-004 — Configuration

- **Title:** `forge.yaml` + `FORGE_*` env vars + `forge config` commands.
- **Status:** implemented
- **Description:** Configuration is layered: `forge.yaml` in the project
  directory (custom_rules, commands, model/provider selection, output
  formatting) and `FORGE_*` environment variables (config dir, log filter,
  search limits, tracker toggle, display currency, MCP globals). The
  `forge config` command group reads/updates values, with `--porcelain` for
  machine-readable output.
- **Acceptance criteria:**
  - A `forge.yaml` with `custom_rules` and `commands` is loaded and applied to
    the session.
  - `FORGE_LOG` filter syntax (`forge=debug`) takes effect.
  - `forge config --porcelain` prints stable key/value pairs parseable by
    scripts.
  - Unknown env vars are ignored without error; malformed `forge.yaml` yields a
    typed error with the offending path.
- **Test location:** `crates/forge_config/src/{config,reader,output,auto_dump}.rs`.

### FR-005 — ZSH plugin

- **Title:** Shell integration: `:command` transformation, completions, theme, doctor.
- **Status:** implemented
- **Description:** `shell-plugin/` ships a ZSH plugin (forge.plugin.zsh,
  forge.theme.zsh, doctor.zsh, setup.zsh, keyboard.zsh) that transforms
  `:command` syntax into forge invocations, provides tab completion and fuzzy
  file/agent tagging, and renders a Terminal-Forge themed right prompt with
  cost/currency. `forge setup`/`forge zsh setup` wire the plugin into `.zshrc`;
  `forge doctor`/`forge zsh doctor` run diagnostics.
- **Acceptance criteria:**
  - `forge setup` updates `.zshrc` with plugin + theme lines (idempotent).
  - `forge doctor` detects missing `fd` and missing `forge` on `$PATH` and
    reports actionable remediation.
  - `:agent_name` tag completion resolves configured agents.
  - Shell completions are generated (`shell-plugin/lib/completion.zsh`) and
    include all top-level subcommands.
- **Test location:** `shell-plugin/lib/{completion,config,context,dispatcher}.zsh`,
  `crates/forge_main/src/zsh/plugin.rs`; acceptance for interactive ZSH
  behavior is manual (`forge zsh doctor`).

### FR-006 — Eval harness

- **Title:** Cross-language evaluation harness (TS executor + YAML tasks).
- **Status:** implemented
- **Description:** `benchmarks/` provides a TypeScript harness (`npm run eval
  <task.yml>`) that executes forge commands in temp dirs with timeout,
  concurrency, CSV data-driven cases, regex/exit-code validation, and
  timestamped debug artifacts. 13 eval suites cover patch/refactor/search
  behaviors.
- **Acceptance criteria:**
  - `npm run eval benchmarks/evals/echo/task.yml` exits 0 with validation
    passing.
  - Parallel execution with concurrency limits does not interleave temp
    workspaces.
  - Failed validations produce timestamped debug artifacts.
  - A `forgee` symlink to the debug binary is honored as the forge binary path.
- **Test location:** `benchmarks/README.md`, `benchmarks/evals/*/task.yml`
  (13 suites), `benchmarks/{cli,task-executor,verification,parse}.ts`.

### FR-007 — Conversation persistence

- **Title:** SQLite session store, resume, and maintenance.
- **Status:** implemented
- **Description:** Conversations persist to a SQLite session store with WAL
  checkpointing, zstd compression of context blobs, FTS/vector search, and
  subagent breadcrumbs. New conversation data is written to a dedicated write
  DB (`~/.forge/.forge.writes.db`); reads union the legacy
  `~/.forge/.forge.db` via a `conversations_all` TEMP VIEW so pre-existing
  sessions remain visible. Override the write target with `FORGE_WRITE_DB_PATH`
  and the legacy read source with `FORGE_LEGACY_DB_PATH`. `forge conversation`
  lists/resumes history; `--conversation-id` resumes a specific session;
  `forge maintenance compress` compresses remaining plaintext blobs
  (idempotent).
- **Acceptance criteria:**
  - A completed session is listed by `forge conversation list` and resumable
    via `--conversation-id`.
  - `forge maintenance compress` reports rows compressed/skipped/failed and is
    safe to re-run.
  - Titles and timestamps remain queryable while blobs are compressed.
  - Credential files remain `0o600` and gitignored (never in the store).
- **Test location:** `crates/forge_main/src/main.rs:217` (conversation tests),
  `crates/forge_dbd/` (daemon), snapshot coverage in
  `crates/forge_app/src/snapshots/`.

### FR-008 — MCP client

- **Title:** Model Context Protocol client (subprocess + HTTP transports).
- **Status:** implemented
- **Description:** MCP servers are managed via `forge mcp {list,import,show,
  remove,reload}` and `.mcp.json` (project-local + `~/.forge/.mcp.json`),
  connecting over subprocess or streamable HTTP via `rmcp`. MCP tools join the
  agent tool registry for multi-agent workflows. Trust gaps are tracked in the
  threat model (G3.1–G3.4).
- **Acceptance criteria:**
  - `forge mcp list` shows configured servers from both config locations.
  - `forge mcp import` accepts a JSON server definition and persists it.
  - `forge mcp reload` rebuilds the server cache without restarting the
    process.
  - MCP tools appear in the agent's tool registry when the server connects.
- **Test location:** `crates/forge_main/src/cli.rs` (McpCommandGroup); threat
  model surface analysis in `docs/security/threat-model.md:149-199`.

### FR-009 — Providers & authentication

- **Title:** Multi-provider model access with local credential custody.
- **Status:** implemented
- **Description:** Providers (OpenAI, Anthropic, Bedrock, neuralwatt, etc.) are
  adapters behind the provider repository; credentials are stored locally with
  `0o600` enforcement, never committed, and resolvable via env vars.
- **Acceptance criteria:**
  - `forge provider` lists configured providers and current auth status.
  - Credential files are created with `0o600` permissions (regression-tested).
  - Unknown providers yield typed configuration errors, not panics.
- **Test location:** `crates/forge_repo/src/provider_repo.rs`,
  `docs/security/threat-model.md:63-69`.

### FR-010 — Self-update

- **Title:** `forge update` with CI/non-interactive safety.
- **Status:** implemented
- **Description:** `forge update` fetches and applies the latest release;
  detects non-interactive contexts (`is_ci`) so CI and pipes never hang on
  prompts.
- **Acceptance criteria:**
  - `forge update` checks the remote and reports current/latest versions.
  - Update runs are skipped or non-interactive when stdin is not a TTY.
- **Test location:** `crates/forge_main/src/update.rs:89-100` (is_ci
  detection), `crates/forge_main/src/main.rs:217`.

### FR-011 — Distribution & install

- **Title:** curl|sh, cargo install, npm, homebrew, Windows install.
- **Status:** implemented
- **Description:** Releases ship a 9-OS binary matrix (`release.yml`),
  `install.sh`/`install.ps1` bootstrap installers, `cargo install
  helioslite --locked`, npm mirror packages, and a Homebrew formula, plus SLSA
  L2 attestation (`release-attestation.yml`).
- **Acceptance criteria:**
  - `install.sh` installs the correct per-OS/arch binary to a writable prefix.
  - `cargo install helioslite --locked` builds from the workspace lockfile.
  - Release artifacts are built with `--locked` and attested (SLSA Build L2).
  - A CycloneDX SBOM is emitted per crate in the release pipeline.
- **Test location:** `install.sh`, `install.ps1`, `.github/workflows/
  release.yml`, `.github/workflows/release-attestation.yml`, `docs/forge-dev-install.md`.

### FR-012 — Semantic search & workspaces

- **Title:** Workspace indexing and semantic search.
- **Status:** implemented
- **Description:** `forge workspace` manages indexed workspaces for semantic
  search; `FORGE_SEM_SEARCH_LIMIT`/`FORGE_SEM_SEARCH_TOP_K` tune vector search;
  forge_similarity/forge_walker/forge_repo_map power code understanding.
- **Acceptance criteria:**
  - A workspace can be created and searched returning results bounded by the
    configured limit.
  - Semantic search quality is covered by the eval suite
    (`benchmarks/evals/semantic_search_quality/`).
- **Test location:** `crates/forge_similarity/`, `benchmarks/evals/
  semantic_search_quality/task.yml`, `benchmarks/evals/sem_search/task.yml`.

### FR-013 — Commit generation

- **Title:** AI-generated commits (`forge commit`).
- **Status:** implemented
- **Description:** `forge commit` generates and optionally commits changes with
  an AI-generated message following repo conventions.
- **Acceptance criteria:**
  - `forge commit` produces a conventional commit message from the staged diff.
  - Generated messages never contain markdown fences (regression-guarded).
- **Test location:** `crates/forge_app/src/snapshots/` (commit_no_markdown
  eval), `benchmarks/evals/commit_no_markdown/task.yml`.

### FR-014 — Accessibility & screen-reader mode (planned)

- **Title:** Terminal-first accessibility: keyboard spec, contrast, SR output.
- **Status:** planned
- **Description:** Explicit accessibility statement and screen-reader-friendly
  output mode for the CLI/TUI. See README "Accessibility" section and
  `docs/VISUAL_SPEC.md` for the statement and planned `--json`/SR surface.
  Terminal-first means color is never the sole carrier of meaning; `NO_COLOR`
  and CI detection are already honored.
- **Acceptance criteria (target):**
  - All status/output that uses color also carries a non-color signal.
  - A keyboard navigation spec exists for TUI widgets.
  - A screen-reader mode emits structured, non-interactive output.
- **Test location:** TBD (blocked on implementation; tracked in C09 gaps).

### FR-015 — Golden-output tests (planned)

- **Title:** Visual regression for CLI/TUI output.
- **Status:** planned
- **Description:** Golden-output tests that pin empty/loading/error state
  rendering per view, per `docs/VISUAL_SPEC.md`. Existing insta snapshots in
  `crates/forge_app/src/snapshots/` are the seed corpus.
- **Acceptance criteria (target):**
  - Each specified view state has a golden reference committed under
    `tests/golden/`.
  - CI fails on unapproved rendering drift (reviewed via snapshot review).
- **Test location:** TBD — planned `tests/golden/` directory (C10 L107 gap).

---

## Coverage matrix

| Cluster | FRs that close gaps |
|---|---|
| C03 Agent Readiness | all FRs (machine-readable acceptance contract) |
| C04 Security | FR-002 (sandbox), FR-008 (MCP trust), FR-009 (credential custody) |
| C09 Accessibility | FR-014 |
| C10 Visual Identity | FR-015, FR-005 (theme) |

Keep this table in sync with the scorecard (`.claude/audit/` outputs) when the
gap set changes.
