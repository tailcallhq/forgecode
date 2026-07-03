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

## Install forge-dev

Grab the latest `forge-dev` binary for your platform:

```sh
curl -sSfL https://github.com/KooshaPari/forgecode/releases/latest/download/install.sh | sh
```

This downloads the correct binary for your OS and architecture (macOS ARM/Intel,
Linux x86_64/ARM64, Windows x86_64), installs it to `/usr/local/bin/forge-dev`
(or `~/.local/bin/forge-dev` if `/usr/local/bin` is not writable), and makes it
executable.

> **Source builds:** To build from source instead, use `cargo build --release
> --features dev-binary --bin forge-dev`. The `forge-dev` binary is the
> fork-specific build of the CLI with Phenotype enhancements.

## Quick Start

```sh
# Run the CLI
cargo run --bin forge-dev --features dev-binary

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

</details>

<details>
<summary><strong>Neuralwatt</strong></summary>

```bash
# .env
NEURALWATT_API_KEY=<your_neuralwatt_api_key>
```

</details>

<details>
<summary><strong>IO Intelligence</strong></summary>

```bash
# .env
IO_INTELLIGENCE_API_KEY=<your_io_intelligence_api_key>
```

```yaml
# forge.yaml
model: meta-llama/Llama-3.3-70B-Instruct
```

</details>

<details>
<summary><strong>OpenAI</strong></summary>

```bash
# .env
OPENAI_API_KEY=<your_openai_api_key>
```

```yaml
# forge.yaml
model: o3-mini-high
```

</details>

<details>
<summary><strong>Anthropic</strong></summary>

```bash
# .env
ANTHROPIC_API_KEY=<your_anthropic_api_key>
```

```yaml
# forge.yaml
model: claude-3.7-sonnet
```

</details>

<details>
<summary><strong>Google Vertex AI</strong></summary>

**Setup Instructions:**

1. **Install Google Cloud CLI** and authenticate:

   ```bash
   gcloud auth login
   gcloud config set project YOUR_PROJECT_ID
   ```

2. **Get your authentication token**:

   ```bash
   gcloud auth print-access-token
   ```

3. **Use the token when logging in via Forge**:

   ```bash
   forge provider login
   # Select Google Vertex AI and enter your credentials
   ```

**Legacy `.env` setup:**

```bash
# .env
PROJECT_ID=<your_project_id>
LOCATION=<your_location>
VERTEX_AI_AUTH_TOKEN=<your_auth_token>
```

```yaml
# forge.yaml
model: google/gemini-2.5-pro
```

**Available Models:**
- Claude models: `claude-sonnet-4@20250514`
- Gemini models: `gemini-2.5-pro`, `gemini-2.0-flash`

Use the `/model` command in Forge CLI to see all available models.

</details>

<details>
<summary><strong>OpenAI-Compatible Providers</strong></summary>

```bash
# .env
OPENAI_API_KEY=<your_provider_api_key>
OPENAI_URL=<your_provider_url>
```

```yaml
# forge.yaml
model: <provider-specific-model>
```

</details>

<details>
<summary><strong>Groq</strong></summary>

```bash
# .env
OPENAI_API_KEY=<your_groq_api_key>
OPENAI_URL=https://api.groq.com/openai/v1
```

```yaml
# forge.yaml
model: deepseek-r1-distill-llama-70b
```

</details>

<details>
<summary><strong>Amazon Bedrock</strong></summary>

To use Amazon Bedrock models with Forge, you'll need to first set up the [Bedrock Access Gateway](https://github.com/aws-samples/bedrock-access-gateway):

1. **Set up Bedrock Access Gateway**:

   - Follow the deployment steps in the [Bedrock Access Gateway repo](https://github.com/aws-samples/bedrock-access-gateway)
   - Create your own API key in Secrets Manager
   - Deploy the CloudFormation stack
   - Note your API Base URL from the CloudFormation outputs

2. **Configure in Forge**:

   ```bash
   forge provider login
   # Select OpenAI-compatible provider and enter your Bedrock Gateway details
   ```

**Legacy `.env` setup:**

```bash
# .env
OPENAI_API_KEY=<your_bedrock_gateway_api_key>
OPENAI_URL=<your_bedrock_gateway_base_url>
```

```yaml
# forge.yaml
model: anthropic.claude-3-opus
```

</details>

<details>
<summary><strong>ForgeCode Services</strong></summary>

```bash
# .env
FORGE_API_KEY=<your_forge_api_key>
```

```yaml
# forge.yaml
model: claude-3.7-sonnet
```

</details>

</details>

---

### forge.yaml Configuration Options

### Environment Variables

Forge supports several environment variables for advanced configuration and fine-tuning. These can be set in your `.env` file or system environment.

<details>
<summary><strong>Retry Configuration</strong></summary>

Control how Forge handles retry logic for failed requests:

```bash
# .env
FORGE_RETRY_INITIAL_BACKOFF_MS=1000    # Initial backoff time in milliseconds (default: 1000)
FORGE_RETRY_BACKOFF_FACTOR=2           # Multiplier for backoff time (default: 2)
FORGE_RETRY_MAX_ATTEMPTS=3             # Maximum retry attempts (default: 3)
FORGE_SUPPRESS_RETRY_ERRORS=false      # Suppress retry error messages (default: false)
FORGE_RETRY_STATUS_CODES=429,500,502   # HTTP status codes to retry (default: 429,500,502,503,504)
```

</details>

<details>
<summary><strong>HTTP Configuration</strong></summary>

Fine-tune HTTP client behavior for API requests:

```bash
# .env
FORGE_HTTP_CONNECT_TIMEOUT=30              # Connection timeout in seconds (default: 30)
FORGE_HTTP_READ_TIMEOUT=900                # Read timeout in seconds (default: 900)
FORGE_HTTP_POOL_IDLE_TIMEOUT=90            # Pool idle timeout in seconds (default: 90)
FORGE_HTTP_POOL_MAX_IDLE_PER_HOST=5        # Max idle connections per host (default: 5)
FORGE_HTTP_MAX_REDIRECTS=10                # Maximum redirects to follow (default: 10)
FORGE_HTTP_USE_HICKORY=false               # Use Hickory DNS resolver (default: false)
FORGE_HTTP_TLS_BACKEND=default             # TLS backend: "default" or "rustls" (default: "default")
FORGE_HTTP_MIN_TLS_VERSION=1.2             # Minimum TLS version: "1.0", "1.1", "1.2", "1.3"
FORGE_HTTP_MAX_TLS_VERSION=1.3             # Maximum TLS version: "1.0", "1.1", "1.2", "1.3"
FORGE_HTTP_ADAPTIVE_WINDOW=true            # Enable HTTP/2 adaptive window (default: true)
FORGE_HTTP_KEEP_ALIVE_INTERVAL=60          # Keep-alive interval in seconds (default: 60, use "none"/"disabled" to disable)
FORGE_HTTP_KEEP_ALIVE_TIMEOUT=10           # Keep-alive timeout in seconds (default: 10)
FORGE_HTTP_KEEP_ALIVE_WHILE_IDLE=true      # Keep-alive while idle (default: true)
FORGE_HTTP_ACCEPT_INVALID_CERTS=false      # Accept invalid certificates (default: false) - USE WITH CAUTION
FORGE_HTTP_ROOT_CERT_PATHS=/path/to/cert1.pem,/path/to/cert2.crt  # Paths to root certificate files (PEM, CRT, CER format), multiple paths separated by commas
```

> **⚠️ Security Warning**: Setting `FORGE_HTTP_ACCEPT_INVALID_CERTS=true` disables SSL/TLS certificate verification, which can expose you to man-in-the-middle attacks. Only use this in development environments or when you fully trust the network and endpoints.

</details>

<details>
<summary><strong>API Configuration</strong></summary>

Override default API endpoints and provider/model settings:

```bash
# .env
FORGE_API_URL=https://api.forgecode.dev  # Custom Forge API URL (default: https://api.forgecode.dev)
FORGE_WORKSPACE_SERVER_URL=http://localhost:8080  # URL for the indexing server (default: https://api.forgecode.dev/)
```

</details>

<details>
<summary><strong>Tool Configuration</strong></summary>

Configuring the tool calls settings:

```bash
# .env
FORGE_TOOL_TIMEOUT=300         # Maximum execution time in seconds for a tool before it is terminated to prevent hanging the session. (default: 300)
FORGE_MAX_IMAGE_SIZE=10485760  # Maximum image file size in bytes for read_image operations (default: 10485760 - 10 MB)
FORGE_DUMP_AUTO_OPEN=false     # Automatically open dump files in browser (default: false)
FORGE_DEBUG_REQUESTS=/path/to/debug/requests.json  # Write debug HTTP request files to specified path (supports absolute and relative paths)
```

</details>

<details>
<summary><strong>ZSH Plugin Configuration</strong></summary>

Configure the ZSH plugin behavior:

```bash
# .env
FORGE_BIN=forge                    # Command to use for forge operations (default: "forge")
```

The `FORGE_BIN` environment variable allows you to customize the command used by the ZSH plugin when transforming `:` prefixed commands. If not set, it defaults to `"forge"`.

</details>

<details>
<summary><strong>Display Configuration</strong></summary>

Configure display options for the Forge UI and ZSH theme:

```bash
# .env
FORGE_CURRENCY_SYMBOL="$"         # Currency symbol for cost display in ZSH theme (default: "$")
FORGE_CURRENCY_CONVERSION_RATE=1.0  # Conversion rate for currency display (default: 1.0)
NERD_FONT=1                       # Enable Nerd Font icons in ZSH theme (default: auto-detected, set to "1" or "true" to enable, "0" or "false" to disable)
USE_NERD_FONT=1                   # Alternative variable for enabling Nerd Font icons (same behavior as NERD_FONT)
```

The `FORGE_CURRENCY_SYMBOL` and `FORGE_CURRENCY_CONVERSION_RATE` variables control how costs are displayed in the ZSH theme right prompt. Use these to customize the currency display for your region or preferred currency.

</details>

<details>
<summary><strong>System Configuration</strong></summary>

System-level environment variables (usually set automatically):

```bash
# .env
FORGE_CONFIG=/custom/config/dir        # Base directory for all Forge config files (default: ~/.forge)
FORGE_MAX_SEARCH_RESULT_BYTES=10240   # Maximum bytes for search results (default: 10240 - 10 KB)
FORGE_HISTORY_FILE=/path/to/history    # Custom path for Forge history file (default: uses system default location)
FORGE_BANNER="Your custom banner text" # Custom banner text to display on startup (default: Forge ASCII art)
FORGE_MAX_CONVERSATIONS=100            # Maximum number of conversations to show in list (default: 100)
FORGE_MAX_LINE_LENGTH=2000             # Maximum characters per line for file read operations (default: 2000)
FORGE_STDOUT_MAX_LINE_LENGTH=2000      # Maximum characters per line for shell output (default: 2000)
SHELL=/bin/zsh                         # Shell to use for command execution (Unix/Linux/macOS)
COMSPEC=cmd.exe                        # Command processor to use (Windows)
```

</details>

<details>
<summary><strong>Semantic Search Configuration</strong></summary>

Configure semantic search behavior for code understanding:

```bash
# .env
FORGE_SEM_SEARCH_LIMIT=200            # Maximum number of results to return from initial vector search (default: 200)
FORGE_SEM_SEARCH_TOP_K=20             # Top-k parameter for relevance filtering during semantic search (default: 20)
```

</details>

<details>
<summary><strong>Logging Configuration</strong></summary>

Configure logging verbosity and output:

```bash
# .env
FORGE_LOG=forge=info                  # Log filter level (default: forge=debug when tracking disabled, forge=info when tracking enabled)
```

The `FORGE_LOG` variable controls the logging level for Forge's internal operations using the standard tracing filter syntax. Common values:
- `forge=error` - Only errors
- `forge=warn` - Warnings and errors
- `forge=info` - Informational messages (default when tracking enabled)
- `forge=debug` - Debug information (default when tracking disabled)
- `forge=trace` - Detailed tracing

</details>

<details>
<summary><strong>Tracking Configuration</strong></summary>

Control tracking of user-identifying metadata in telemetry events:

```bash
# .env
FORGE_TRACKER=false                   # Disable tracking enrichment metadata (default: true)
```

The `FORGE_TRACKER` variable controls whether tracking enrichment metadata is included in telemetry events.

</details>

The `forge.yaml` file supports several advanced configuration options that let you customize Forge's behavior.

<details>
<summary><strong>Custom Rules</strong></summary>

Add your own guidelines that all agents should follow when generating responses.

```yaml
# forge.yaml
custom_rules: |
  1. Always add comprehensive error handling to any code you write.
  2. Include unit tests for all new functions.
  3. Follow our team's naming convention: camelCase for variables, PascalCase for classes.
```

</details>

<details>
<summary><strong>Commands</strong></summary>

Define custom commands as shortcuts for repetitive prompts:

```yaml
# forge.yaml
commands:
  - name: "refactor"
    description: "Refactor selected code"
    prompt: "Please refactor this code to improve readability and performance"
```

</details>

<details>
<summary><strong>Model</strong></summary>

Specify the default AI model to use for all agents in the workflow.

```yaml
# forge.yaml
model: "claude-3.7-sonnet"
```

</details>

<details>
<summary><strong>Max Walker Depth</strong></summary>

Control how deeply Forge traverses your project directory structure when gathering context.

```yaml
# forge.yaml
max_walker_depth: 3 # Limit directory traversal to 3 levels deep
```

</details>

<details>
<summary><strong>Temperature</strong></summary>

Adjust the creativity and randomness in AI responses. Lower values (0.0-0.3) produce more focused, deterministic outputs, while higher values (0.7-2.0) generate more diverse and creative results.

```yaml
# forge.yaml
temperature: 0.7 # Balanced creativity and focus
```

</details>
<details>
<summary><strong>Tool Max Failure Limit</strong></summary>

Control how many times a tool can fail before Forge forces completion to prevent infinite retry loops. This helps avoid situations where an agent gets stuck repeatedly trying the same failing operation.

```yaml
# forge.yaml
max_tool_failure_per_turn: 3 # Allow up to 3 failures per tool before forcing completion
```

Set to a higher value if you want more retry attempts, or lower if you want faster failure detection.

</details>

<details>
<summary><strong>Max Requests Per Turn</strong></summary>

Limit the maximum number of requests an agent can make in a single conversation turn. This prevents runaway conversations and helps control API usage and costs.

```yaml
# forge.yaml
max_requests_per_turn: 50 # Allow up to 50 requests per turn
```

When this limit is reached, Forge will:

- Ask you if you wish to continue
- If you respond with 'Yes', it will continue the conversation
- If you respond with 'No', it will end the conversation

</details>

---

<details>
<summary><strong>Model Context Protocol (MCP)</strong></summary>

The MCP feature allows AI agents to communicate with external tools and services. This implementation follows Anthropic's [Model Context Protocol](https://docs.anthropic.com/en/docs/claude-code/tutorials#set-up-model-context-protocol-mcp) design.

### MCP Configuration

Configure MCP servers using the CLI:

```bash
# List all MCP servers
forge mcp list

# Import a server from JSON
forge mcp import

# Show server configuration details
forge mcp show

# Remove a server
forge mcp remove

# Reload servers and rebuild caches
forge mcp reload
```

Or manually create a `.mcp.json` file with the following structure:

```json
{
  "mcpServers": {
    "server_name": {
      "command": "command_to_execute",
      "args": ["arg1", "arg2"],
      "env": { "ENV_VAR": "value" }
    },
    "another_server": {
      "url": "http://localhost:3000/events"
    }
  }
}
```

MCP configurations are read from two locations (project-local takes precedence):

1. **Project-local:** `.mcp.json` in your project directory
2. **Global:** `~/forge/.mcp.json`

### Example Use Cases

MCP can be used for various integrations:

- Web browser automation
- External API interactions
- Tool integration
- Custom service connections

### Usage in Multi-Agent Workflows

MCP tools can be used as part of multi-agent workflows, allowing specialized agents to interact with external systems as part of a collaborative problem-solving approach.

</details>

---

## Documentation

For comprehensive documentation on all features and capabilities, please visit the [documentation site](https://github.com/tailcallhq/forgecode/tree/main/docs).

---

## Installation

```bash
# YOLO
curl -fsSL https://forgecode.dev/cli | sh

# Package managers
nix run github:tailcallhq/forgecode # for latest dev branch
```

---

## Community

Join our vibrant Discord community to connect with other Forge users and contributors, get help with your projects, share ideas, and provide feedback!

[![Discord](https://img.shields.io/discord/1044859667798568962?style=for-the-badge&cacheSeconds=120&logo=discord)](https://discord.gg/kRZBPpkgwq)

---

Credentials are stored locally at `~/.forge` / `.credentials.json` with `0o600` permissions and are gitignored. Never commit credentials; use environment variables or the local credential store.

## Contributing

Read `CLAUDE.md` and `AGENTS.md` first — they are the canonical contributor contract. CI gates on `cargo fmt --check`, `cargo clippy -D warnings`, and the test suite (Linux runner).
