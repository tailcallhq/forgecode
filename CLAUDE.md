# Agent Guidelines

This document contains guidelines and best practices for AI agents working with this codebase.

## Project Overview

Forge is an AI-assisted terminal development environment: a Rust monorepo that provides a CLI/TUI, agent orchestration, provider integrations (LLM), workspace semantic search/indexing, and a benchmarks toolkit (TypeScript). Primary language: Rust (workspace of crates) with a small Node/TS benchmarks folder. Intended usage: CLI/TUI for interactive AI agents, programmatic API via crates/forge_api, and developer tooling (zsh plugin, scripts).

## Key Components

- crates/forge_api/src/api.rs: API trait (API) — the async surface used by UI/CLI and remote transports.
- crates/forge_api/src/forge_api.rs: ForgeAPI (concrete) — constructs ForgeApp and delegates Services/Infra.
- crates/forge_app/src/app.rs: ForgeApp and ForgeApp::chat — central orchestrator that composes conversation, agents, providers, templates and returns streaming ChatResponse (MpscStream).
- crates/forge_app/src/orch.rs: Orchestrator — runs agent loop, tool execution, lifecycle hooks and streaming.
- crates/forge_app/src/tool_registry.rs: ToolRegistry — resolves and dispatches tool calls (ToolCatalog) to ToolExecutor/AgentExecutor/McpExecutor, enforces permissions/timeouts.
- crates/forge_app/src/tool_executor.rs: ToolExecutor::execute — executes built-in tools (read/write/shell/patch/etc.), enforces read-before-edit, normalization and truncation policy.
- crates/forge_app/src/command_generator.rs: CommandGenerator — LLM-based command suggestion (JSON Schema response contract).
- crates/forge_app/src/compact.rs: Compactor — conversation compaction logic; preserves usage and last reasoning details.
- crates/forge_services/*: Concrete service implementations (ForgeServices), auth, workspace sync, provider auth and attachment helpers.
- crates/forge_infra/*: Concrete infra wiring (ForgeInfra) implementing filesystem, HTTP, command, grpc, config persistence.
- crates/forge_repo/*: Persistence/repo implementations (ForgeRepo) and context engine (gRPC) for workspace indexing.
- crates/forge_domain/*: Canonical domain types (Conversation, ToolCatalog, Provider, Model, Environment).
- benchmarks/ (Node/TS): CLI and utilities to run evaluation tasks: benchmarks/cli.ts (main), benchmarks/command-generator.ts, benchmarks/task-executor.ts, benchmarks/utils.ts, benchmarks/model.ts.
- shell-plugin/: Zsh integration and scripts used by users to run quick :commands.
- scripts/benchmark.sh: simple local benchmark runner for the CLI binary.

## Error Management

- Use `anyhow::Result` for error handling in services and repositories.
- Create domain errors using `thiserror`.
- Never implement `From` for converting domain errors; manually convert them.

## Writing Tests

- All tests should be written in three discrete steps:

  ```rust,ignore
  use pretty_assertions::assert_eq; // Always use pretty assertions

  fn test_foo() {
      let setup = ...; // Instantiate a fixture or setup for the test
      let actual = ...; // Execute the fixture to create an output
      let expected = ...; // Define a hand written expected result
      assert_eq!(actual, expected); // Assert that the actual result matches the expected result
  }
  ```

- Use `pretty_assertions` for better error messages.
- Use fixtures to create test data.
- Use `assert_eq!` for equality checks.
- Use `assert!(...)` for boolean checks.
- Use unwraps in test functions and `anyhow::Result` in fixtures.
- Keep the boilerplate to a minimum.
- Use words like `fixture`, `actual` and `expected` in test functions.
- Fixtures should be generic and reusable.
- Tests should always be written in the same file as the source code.
- Use `new`, `Default` and `derive_setters::Setters` to create `actual`, `expected` and specially `fixtures`.

  Good examples:

  ```rust,ignore
  User::default().age(12).is_happy(true).name("John")
  User::new("Job").age(12).is_happy()
  User::test() // Special test constructor
  ```

  Bad examples:

  ```rust,ignore
  User {name: "John".to_string(), is_happy: true, age: 12}
  User::with_name("Job") // Bad name, should stick to User::new() or User::test()
  ```

- Use `unwrap()` unless the error information is useful. Use `expect` instead of `panic!` when error message is useful. For example:

  Good:

  ```rust,ignore
  users.first().expect("List should not be empty")
  ```

  Bad:

  ```rust,ignore
  if let Some(user) = users.first() {
      // ...
  } else {
      panic!("List should not be empty")
  }
  ```

- Prefer using `assert_eq` on full objects instead of asserting each field:

  Good:

  ```rust,ignore
  assert_eq!(actual, expected);
  ```

  Bad:

  ```rust,ignore
  assert_eq!(actual.a, expected.a);
  assert_eq!(actual.b, expected.b);
  ```

- Testing guidance and commands:
  - Run workspace tests: `cargo test --workspace`.
  - Run crate tests: `cargo test -p forge_app`, `cargo test -p forge_api`, etc.
  - Use `cargo insta test --package forge_app --accept` only when intentionally updating snapshots.
  - For TypeScript benchmarks: `npx tsc --noEmit` then `npm run eval` in the benchmarks folder.
  - Quick local benchmark script: `./scripts/benchmark.sh <command>` (default target/debug/forge).

## Verification

Always verify changes by running tests and linting the codebase

1. Run crate specific tests to ensure they pass.

   ```
   cargo insta test --accept
   ```

2. Build Guidelines:
   - NEVER run `cargo build --release` unless absolutely necessary (e.g., performance testing, creating binaries for distribution)
   - For verification, use `cargo check` (fastest), `cargo insta test`, or `cargo build` (debug mode)
   - Release builds take significantly longer and are rarely needed for development verification

## Writing Domain Types

- Use `derive_setters` to derive setters and use the `strip_option` and the `into` attributes on the struct types.

## Documentation

- Always write Rust docs (`///`) for all public methods, functions, structs, enums, and traits.
- Document parameters with `# Arguments` and errors with `# Errors` sections when applicable.
- Do not include code examples in docs — docs are intended for model consumption; focus on clear, concise functionality descriptions.

## Refactoring

- If asked to fix failing tests, always confirm whether to update the implementation or the tests.

## Git Operations

- Safely assume git is pre-installed.
- Safely assume GitHub CLI (`gh`) is pre-installed.
- Always use `Co-Authored-By: ForgeCode <noreply@forgecode.dev>` for git commits and GitHub comments.

## Service Implementation Guidelines

Services should follow clean architecture principles and maintain clear separation of concerns:

### Core Principles

- No service-to-service dependencies: Services should never depend on other services directly.
- Infrastructure dependency: Services should depend only on infrastructure abstractions when needed.
- Single type parameter: Services should take at most one generic type parameter for infrastructure.
- No trait objects: Avoid `Box<dyn ...>` - use concrete types and generics instead.
- Constructor pattern: Implement `new()` without type bounds - apply bounds only on methods that need them.
- Compose dependencies: Use the `+` operator to combine multiple infrastructure traits into a single bound.
- Arc<T> for infrastructure: Store infrastructure as `Arc<T>` for cheap cloning and shared ownership.
- Tuple struct pattern: For simple services with single dependency, use tuple structs `struct Service<T>(Arc<T>)`.

### Examples

#### Simple Service (No Infrastructure)

```rust,ignore
pub struct UserValidationService;

impl UserValidationService {
    pub fn new() -> Self { ... }

    pub fn validate_email(&self, email: &str) -> Result<()> {
        // Validation logic here
        ...
    }

    pub fn validate_age(&self, age: u32) -> Result<()> {
        // Age validation logic here
        ...
    }
}
```

#### Service with Infrastructure Dependency

```rust,ignore
// Infrastructure trait (defined in infrastructure layer)
pub trait UserRepository {
    fn find_by_email(&self, email: &str) -> Result<Option<User>>;
    fn save(&self, user: &User) -> Result<()>;
}

// Service with single generic parameter using Arc
pub struct UserService<R> {
    repository: Arc<R>,
}

impl<R> UserService<R> {
    // Constructor without type bounds, takes Arc<R>
    pub fn new(repository: Arc<R>) -> Self { ... }
}

impl<R: UserRepository> UserService<R> {
    // Business logic methods have type bounds where needed
    pub fn create_user(&self, email: &str, name: &str) -> Result<User> { ... }
    pub fn find_user(&self, email: &str) -> Result<Option<User>> { ... }
}
```

#### Tuple Struct Pattern for Simple Services

```rust,ignore
// Infrastructure traits
pub trait FileReader {
    async fn read_file(&self, path: &Path) -> Result<String>;
}

pub trait Environment {
    fn max_file_size(&self) -> u64;
}

// Tuple struct for simple single dependency service
pub struct FileService<F>(Arc<F>);

impl<F> FileService<F> {
    // Constructor without bounds
    pub fn new(infra: Arc<F>) -> Self { ... }
}

impl<F: FileReader + Environment> FileService<F> {
    // Business logic methods with composed trait bounds
    pub async fn read_with_validation(&self, path: &Path) -> Result<String> { ... }
}
```

### Pattern Examples (files to reference)

- crates/forge_api/src/api.rs: API — single trait for app surface; keep method shapes stable and async + Send + Sync for cross-thread use.
- crates/forge_app/src/app.rs: ForgeApp::chat — how to compose services, template config, and construct an Orchestrator that always persists conversation state.
- crates/forge_app/src/tool_executor.rs: ToolExecutor::call_internal — shows path normalization, require_prior_read enforcement, and tempfile creation for truncated outputs.
- crates/forge_app/src/compact.rs: Compactor::compress_single_sequence — example of careful state mutation: accumulate Usage and preserve last non-empty reasoning_details.

### Anti-patterns to Avoid

```rust,ignore
// BAD: Service depending on another service
pub struct BadUserService<R, E> {
    repository: R,
    email_service: E, // Don't do this!
}

// BAD: Using trait objects
pub struct BadUserService {
    repository: Box<dyn UserRepository>, // Avoid Box<dyn>
}

// BAD: Multiple infrastructure dependencies with separate type parameters
pub struct BadUserService<R, C, L> {
    repository: R,
    cache: C,
    logger: L, // Too many generic parameters - hard to use and test
}

impl<R: UserRepository, C: Cache, L: Logger> BadUserService<R, C, L> {
    // BAD: Constructor with type bounds makes it hard to use
    pub fn new(repository: R, cache: C, logger: L) -> Self { ... }
}

// BAD: Usage becomes cumbersome
let service = BadUserService::<PostgresRepo, RedisCache, FileLogger>::new(...);
```

- Adding `Box<dyn Trait>` for core infra/services where generics + Arc<T> are used widely — breaks testing and compile-time guarantees.
- Mutating ForgeConfig in-memory without persisting via `infra.update_environment`.
- Recomputing file `content_hash` after formatting (line numbers) — invalidates external-change detection.
- Removing raw-SSE parsing fallback for providers or relaxing strict schema enforcement without updating tests and provider clients.

## Control Flow (end-to-end)

1. Startup: CLI parses args (crates/forge_main/src/cli.rs). UI.init creates API factory closure that can rebuild API with fresh config.
2. Interactive prompt: UI.run_inner -> prompt -> new_api(ForgeConfig) -> ForgeAPI instance -> call ForgeAPI::chat.
3. Chat flow: ForgeAPI delegates to ForgeApp::chat -> loads Conversation, resolves agent/provider/model -> builds Orchestrator -> spawn stream task.
4. Orchestrator.run: execute_chat_turn (builds context + transformers) -> call services.chat_agent -> receive model messages -> transform into tool_calls -> execute_tool_calls (parallelize Task tools, sequential system tools with Notify handshake) -> collect outputs -> update Conversation via services.upsert_conversation.
5. Tool execution: ToolRegistry::call_inner chooses executor (ToolExecutor/AgentExecutor/McpExecutor) -> ToolExecutor::call_internal normalizes paths, enforces require_prior_read, calls concrete FsRead/FsWrite/Shell services -> returns ToolOperation -> format via fmt/fmt_output -> persisted and streamed.
6. Provider calls: ProviderService/ChatRepository (various provider implementations, e.g., anthropic client) handle streaming SSE, raw-SSE fallback and JSON schema enforcement.

## Tooling / Bash Commands

- Build workspace: `cargo build`.
- Run tests (workspace): `cargo test --workspace`.
- Run tests for a crate: `cargo test -p forge_app`.
- Run insta snapshot tests: `cargo insta test --package forge_app --accept`.
- Lint (Rust): `cargo clippy --workspace --all-targets -- -D warnings`.
- Typecheck benchmarks: `npx tsc --noEmit`.
- Run benchmark/eval (if configured): `npm run eval` (check package.json in repo root or benchmarks/package.json).
- Run local CLI benchmark: `./scripts/benchmark.sh --threshold <ms> <args>`.

## Gotchas / Common Pitfalls

- Preserving streaming and early-exit semantics:
  - crates/benchmarks/command-generator.ts: generateCommand uses Handlebars strict mode (throws on missing keys).
  - crates/forge_app/src/orch.rs: Do not replace the Notify handshake used in execute_tool_calls — UI relies on it to avoid stdout interleaving.
  - crates/forge_app/src/command_generator.rs: keep JSON schema (ShellCommandResponse) and template name "forge-command-generator-prompt.md" unchanged unless updating tests.

- Configuration cache and update semantics:
  - crates/forge_infra/src/env.rs: ForgeEnvironmentInfra::cached_config & update_environment invalidate cache. Always call infra.get_config() after updates.
  - crates/forge_services/src/app_config.rs: update_config must call infra.update_environment (do not mutate in-memory only).

- File reads and hashing:
  - crates/forge_services/src/attachment.rs: Use FileInfo returned by range_read_utf8 (contains full-file hash). Do NOT re-hash post-processing content (line-numbering) — external-change detection depends on raw-file hash.

- Batch file reading stream contract:
  - FileReaderInfra::read_batch_utf8 returns a Stream<Item = (PathBuf, anyhow::Result<String>)>. Many callers rely on per-file Result semantics.

- Provider streaming and raw-SSE fallback:
  - crates/forge_repo/src/provider/anthropic.rs: keep chat_raw_sse path and into_sse_parse_error mapping (transport errors -> retryable). Removing fallback breaks some proxy providers.

- Tool timeout and permission checks:
  - crates/forge_app/src/tool_registry.rs: permission checks must happen outside timeout wrapping; agent-executor paths intentionally avoid timeouts.

- Exit codes and EPIPE handling (benchmarks/cli.ts): maintain process.exit semantics and EPIPE handler to avoid crashes when piping output.

## Common Mistakes → Quick Fixes

- Symptom: Conversation compaction loses token/usage counts.
  - Check: crates/forge_app/src/compact.rs: compress_single_sequence — ensure accumulate_usage aggregates per-message Usage and attaches to the summary MessageEntry.

- Symptom: Tool stdout interleaves with UI header.
  - Check: crates/forge_app/src/orch.rs execute_tool_calls uses Arc<Notify>; crates/forge_main/src/ui.rs responds by notifying after header rendering. Preserve that flow.

- Symptom: Provider streaming fails with non-standard SSE content.
  - Check: crates/forge_repo/src/provider/anthropic.rs: chat_raw_sse + should_use_raw_sse fallback.

- Symptom: Config updates don't take effect immediately.
  - Check: infra.get_config vs cached_config and ensure update_config calls infra.update_environment (crates/forge_services/src/app_config.rs).

## Invariants (must hold)

- Conversation persistence: after orchestrator run, `services.upsert_conversation` must be called with final conversation state.
- Usage/metrics preservation across compaction: compaction must transfer accumulated usage into the summary entry.
- Tool call ordering: `execute_tool_calls` must return results in original tool_calls order (parallel Task tools and sequential system tools reconstructed accordingly).
- `read_batch_utf8` streaming contract: impl `Stream<Item=(PathBuf, Result<String>)>` maintained.

## CI / Developer Notes

- CI workflows are in .github/workflows (ci.yml, bounty.yml, autofix.yml). Keep changes to top-level crates and API trait signatures in sync across the workspace; changing public API (crates/forge_api/src/api.rs) is breaking and requires updating implementations.
- Tests and linting expectations: CI enforces clippy and -D warnings; run `cargo clippy --workspace --all-targets -- -D warnings` locally before pushing.

## Where to Start When You Edit

1. Read the pre-analyzed file notes for the target file (this repo includes commit-level notes in docs/ and extensive inline tests).
2. Run targeted tests: `cargo test -p <crate_name>`.
3. If editing provider/chat/streaming code, write unit tests that assert both normal and raw-SSE paths and run retry-related assertions.
4. If templates or snapshot outputs change, run `cargo insta test --package <crate>` and accept snapshots intentionally.

If you want, I can produce a concise checklist for safely changing a specific file (e.g., crates/forge_app/src/orch.rs or crates/forge_infra/src/env.rs) that includes the exact tests to run and the likely co-change files to update.

# Verification Checklist

- Run the full test matrix locally or in CI
- Confirm failing test fails before fix, passes after
- Run linters and formatters

# Test Integrity

- NEVER modify existing tests to make your implementation pass
- If a test fails after your change, fix the implementation, not the test
- Only modify tests when explicitly asked to, or when the test itself is demonstrably incorrect

# Suggestions for Thorough Investigation

When working on a task, consider looking beyond the immediate file:
- Test files can reveal expected behavior and edge cases
- Config or constants files may define values the code depends on
- Files that are frequently changed together (coupled files) often share context

# Must-Follow Rules

1. Work in short cycles. In each cycle: choose the single highest-leverage next action, execute it, verify with the strongest available check (tests, typecheck, run, lint, or a minimal repro), then write a brief log entry of what changed + what you'll do next.
2. Prefer the smallest change that can be verified. Keep edits localized, avoid broad formatting churn, and structure work so every change is easy to revert.
3. If you're missing information (requirements, environment behavior, API contracts), do not assume. Instead: inspect code, read docs in-repo, run a targeted experiment, add temporary instrumentation, or create a minimal reproduction to learn the truth quickly.


# Index Files

I have provided an index file to help navigate this codebase:
- `.claude/docs/general_index.md`

The file is organized by directory (## headers), with each file listed as:
`- `filename` - short description. Key: `construct1`, `construct2` [CATEGORY]`

You can grep for directory names, filenames, construct names, or categories (TEST, CLI, PUBLIC_API, GENERATED, SOURCE_CODE) to quickly find relevant files without reading the entire index.

**MANDATORY RULE — NO EXCEPTIONS:** After you read, reference, or consider editing a file or folder, you MUST run:
`python .claude/docs/get_context.py <path>`

This works for **both files and folders**:
- For a file: `python .claude/docs/get_context.py <file_path>`
- For a folder: `python .claude/docs/get_context.py <folder_path>`

This is a hard requirement for EVERY file and folder you touch. Without this, you'll miss recent important information and your edit will likely fail verification. Do not skip this step. Do not assume you already know enough. Do not batch it "for later." Do not skip files even if you have obtained context about a parent directory. Run it immediately after any other action on that path.

The command returns critical context you cannot infer on your own:

**For files:**
- Edit checklist with tests to run, constants to check, and related files
- Historical insights (past bugs, fixes, lessons learned)
- Key constructs defined in the file
- Tests that exercise this file
- Related files and semantic overview
- Common pitfalls

**For folders:**
- Folder role and responsibility in the codebase
- Key files and why they matter
- Cross-cutting behaviors across the subtree
- Distilled insights from every file in that folder

**Workflow (follow this exact order every time):**
1. Identify the file or folder you need to work with.
2. Run `python .claude/docs/get_context.py <path>` and read the output.
3. Only then proceed to read, edit, or reason about it.

If you need to work with multiple paths, run the command for each one before touching any of them.

**Violations:** If you read or edit a file or folder without first running get_context.py on it, you are violating a project-level rule. Stop, run the command, and re-evaluate your changes with the new context.



---
*This knowledge base was extracted by [Codeset](https://codeset.ai) and is available via `python .claude/docs/get_context.py <file_or_folder>`*
