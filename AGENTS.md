# Agent Guidelines

This document contains guidelines and best practices for AI agents working with this codebase.

## Custom Commands (Slash Commands)

User-defined slash commands live as YAML-frontmatter `.md` files in `.forge/commands/`.
Built-in commands live in `commands/`. Commands are loaded by `CommandLoaderService`
(`crates/forge_services/src/command.rs`) and exposed via `/command_name` in the TUI/REPL.

**Resolution priority**:

1. User-defined (`.forge/commands/`) overrides built-in
2. Built-in (`commands/`)

**Built-in commands**:

- `github-pr-description` — Generate a PR description from git diff

**User-defined commands (`.forge/commands/`)**:

| Command      | Description                                     |
| ------------ | ----------------------------------------------- |
| `review`     | Request code review of selected files           |
| `test`       | Author or run tests for selected code           |
| `think`      | Extended analysis / deep-think mode             |
| `exec`       | Execute a shell command and capture output      |
| `pipe`       | Pipe input/output between tools                 |
| `capture`    | Capture command output for context              |
| `check`      | Run lint/type checks                            |
| `fixme`      | Show and resolve fixme/todo comments            |
| `parent`     | Send message to parent conversation             |
| `subagents`  | Delegate work to sub-agents                     |

Each command file uses YAML frontmatter (`name`, `description`, optional `parameters`)
with a Markdown body as the prompt template. The `Command` domain model is in
`forge_domain::command::Command`.

## Resilience & Stability Patterns

### Retry Policy

Defined in `docs/contracts/provider-models/resilience-policy.schema.json`.
Implemented in `forge_config::RetryConfig` + `forge_app::retry` (backon crate).

| Parameter            | Default | Description                           |
| -------------------- | ------- | ------------------------------------- |
| `max_attempts`       | 8       | Total attempts (initial + 7 retries)  |
| `initial_backoff_ms` | 200     | Base delay before first retry         |
| `min_delay_ms`       | 1000    | Floor on computed backoff             |
| `backoff_factor`     | 2.0     | Exponential multiplier per attempt    |
| `max_delay_secs`     | null    | Optional cap on backoff               |
| `jitter`             | true    | Uniform random jitter                 |
| `suppress_errors`    | false   | Suppress logging for non-critical ops |

**Retryable HTTP codes**: 408, 429, 500, 502, 503, 504, 520, 522, 524, 529.
**Non-retryable**: 400, 401, 403, 404, 405, 409, 410, 413, 422, 451.

### Circuit Breaker (Tool Error Tracker)

`ToolErrorTracker` in the orchestrator tolerates up to `max_tool_failure_per_turn`
(default 3) errors per agent turn before propagating failure. Prevents cascading
tool errors from crashing the agent loop.

### Doom-Loop Detection

`crates/forge_app/src/hooks/doom_loop.rs`: detects repetitive tool-call patterns
and injects a reminder prompt to break the cycle.

### Compaction Strategy

`forge_domain::compact/`: sliding-window summarization with adaptive eviction and
importance scoring. Configurable per agent with `CompactionConfig`.

### Hook Chain

Chainable lifecycle hooks: `on_start`, `on_request`, `on_response`, `on_toolcall_start`,
`on_toolcall_end`, `on_end`. Built-in handlers include CompactionHandler,
DoomLoopDetector, PendingTodosHandler, TitleGenerationHandler, TracingHandler.

## Error Management

- Use `anyhow::Result` for error handling in services and repositories.
- Create domain errors using `thiserror`.
- Never implement `From` for converting domain errors, manually convert them

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

- Use unwraps in test functions and anyhow::Result in fixtures.

- Keep the boilerplate to a minimum.

- Use words like `fixture`, `actual` and `expected` in test functions.

- Fixtures should be generic and reusable.

- Test should always be written in the same file as the source code.

- Use `new`, Default and derive_setters::Setters to create `actual`, `expected` and specially `fixtures`. For example:

  **Good:**

  ```rust,ignore
  User::default().age(12).is_happy(true).name("John")
  User::new("Job").age(12).is_happy()
  User::test() // Special test constructor
  ```

  **Bad:**

  ```rust,ignore
  User {name: "John".to_string(), is_happy: true, age: 12}
  User::with_name("Job") // Bad name, should stick to User::new() or User::test()
  ```

- Use `unwrap()` unless the error information is useful. Use `expect` instead of `panic!` when error message is useful. For example:

  **Good:**

  ```rust,ignore
  users.first().expect("List should not be empty")
  ```

  **Bad:**

  ```rust,ignore
  if let Some(user) = users.first() {
      // ...
  } else {
      panic!("List should not be empty")
  }
  ```

- Prefer using `assert_eq` on full objects instead of asserting each field:

  **Good:**

  ```rust,ignore
  assert_eq!(actual, expected);
  ```

  **Bad:**

  ```rust,ignore
  assert_eq!(actual.a, expected.a);
  assert_eq!(actual.b, expected.b);
  ```

## Verification

Always verify changes by running tests and linting the codebase

1. Run crate specific tests to ensure they pass.

   ```
   cargo insta test --accept
   ```

2. **Build Guidelines**:
   - **NEVER** run `cargo build --release` unless absolutely necessary (e.g., performance testing, creating binaries for distribution)
   - For verification, use `cargo check` (fastest), `cargo insta test`, or `cargo build` (debug mode)
   - Release builds take significantly longer and are rarely needed for development verification

## Writing Domain Types

- Use `derive_setters` to derive setters and use the `strip_option` and the `into` attributes on the struct types.

## Documentation

- **Always** write Rust docs (`///`) for all public methods, functions, structs, enums, and traits.
- Document parameters with `# Arguments` and errors with `# Errors` sections when applicable.
- **Do not include code examples** - docs are for LLMs, not humans. Focus on clear, concise functionality descriptions.

## Refactoring

- If asked to fix failing tests, always confirm whether to update the implementation or the tests.

## Git Operations

- Safely assume git is pre-installed
- Safely assume github cli (gh) is pre-installed
- Always use `Co-Authored-By: ForgeCode <noreply@forgecode.dev>` for git commits and Github comments

## Service Implementation Guidelines

Services should follow clean architecture principles and maintain clear separation of concerns:

### Core Principles

- **No service-to-service dependencies**: Services should never depend on other services directly
- **Infrastructure dependency**: Services should depend only on infrastructure abstractions when needed
- **Single type parameter**: Services should take at most one generic type parameter for infrastructure
- **No trait objects**: Avoid `Box<dyn ...>` - use concrete types and generics instead
- **Constructor pattern**: Implement `new()` without type bounds - apply bounds only on methods that need them
- **Compose dependencies**: Use the `+` operator to combine multiple infrastructure traits into a single bound
- **Arc<T> for infrastructure**: Store infrastructure as `Arc<T>` for cheap cloning and shared ownership
- **Tuple struct pattern**: For simple services with single dependency, use tuple structs `struct Service<T>(Arc<T>)`

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
