# ForgeCode Threat Model (STRIDE)

> Status: Living document. Owner: ForgeCode maintainers.
> Last reviewed: 2026-06-28 (Phase P4 overhaul).
> Scope: the `forge` CLI/TUI binary, the `forge_dbd` local daemon, the ZSH
> shell integration, and all credential/telemetry/MCP surfaces they expose.

ForgeCode is a **local, single-user developer tool**, not a hosted
multi-tenant service. The trust boundary is the user's own machine and OS
account. The primary adversaries we model are therefore:

- **A1 — Local malware / other local users** with read access to the user's
  files but *not* the user's running process memory.
- **A2 — A malicious or compromised remote endpoint** (LLM provider, MCP
  server, OAuth IdP) that returns hostile data.
- **A3 — A malicious model / prompt-injection payload** that steers the agent
  into running attacker-chosen tools or shell commands.
- **A4 — A passive observer** of forge's outbound network traffic (telemetry,
  provider calls).

Out of scope: kernel/firmware compromise, an attacker who already has the
user's interactive shell (they already have everything forge has), supply-chain
attacks on `cargo` dependencies (tracked separately via `cargo audit` / SBOM).

## Attack surfaces

| # | Surface | Crate / file (evidence) |
|---|---------|-------------------------|
| S1 | Credential store (`.credentials.json`, OAuth tokens) | `crates/forge_repo/src/provider/provider_repo.rs`, `crates/forge_domain/src/env.rs`, `crates/forge_infra/src/auth/mcp_credentials.rs` |
| S2 | Prompt injection → tool / subprocess execution | `crates/forge_app/src/tool_registry.rs`, `crates/forge_pheno_shell`, `crates/forge_infra/src/mcp_client.rs` |
| S3 | MCP server trust (untrusted MCP servers) | `crates/forge_infra/src/mcp_client.rs`, `crates/forge_domain/src/mcp.rs` |
| S4 | Telemetry egress (PostHog) | `crates/forge_tracker/src/collect/posthog.rs`, `crates/forge_tracker/src/dispatch.rs`, `crates/forge_tracker/src/can_track.rs` |
| S5 | ZSH plugin / shell integration | `shell-plugin/forge.plugin.zsh`, `shell-plugin/forge.setup.zsh`, `shell-plugin/lib/*.zsh` |

A supporting surface is the **local daemon** `forge_dbd` (Unix domain socket at
`~/.forge/.forge.db.sock`, SQLite store at `~/.forge/forge.db`); it is analyzed
inline within S1/S2 (DoS) since it shares the credential-directory trust model.
Client conversation storage is split by default: writes go to
`~/.forge/.forge.writes.db` and reads union the legacy `~/.forge/.forge.db`
via the `conversations_all` TEMP VIEW.

---

## S1 — Credential store

OAuth tokens, API keys, and MCP client registrations are persisted to disk
under `~/.forge/`. Provider credentials live in `.credentials.json`
(`crates/forge_domain/src/env.rs:173-175`); MCP OAuth state in
`.mcp-credentials.json` (`crates/forge_infra/src/auth/mcp_credentials.rs:117-119`).

### STRIDE

- **Spoofing** — A token file forged by another user could impersonate the
  victim to the provider.
- **Tampering** — Editing `.credentials.json` to point at an attacker
  refresh-token endpoint, or to inject a malicious `base_url`.
- **Repudiation** — Low: single-user tool, no audit of who wrote the file.
- **Information disclosure** — *Primary risk.* Long-lived OAuth refresh tokens
  and API keys readable by other local processes/users, or leaked via logs.
- **Denial of service** — Corrupting the file blocks all authenticated calls.
- **Elevation of privilege** — A stolen refresh token grants the attacker the
  user's full provider quota and any provider-side scopes.

### Current mitigations (evidence-cited)

- **0o600 (owner read/write only)** is enforced on credential files on Unix
  after every write:
  - `crates/forge_repo/src/provider/provider_repo.rs:609-616`
    (`set_owner_only_permissions` → `perms.set_mode(0o600)`), with a regression
    test at `provider_repo.rs:1308-1337` asserting mode `0o600`.
  - `crates/forge_infra/src/auth/mcp_credentials.rs:103-110` applies the same
    0o600 mode to `.mcp-credentials.json`.
- **Secret-Debug redaction** prevents tokens/keys from leaking into logs,
  panics, or telemetry payloads. Custom `Debug` impls render `<redacted>`:
  - `crates/forge_domain/src/auth/new_types.rs:9-12` — `ApiKey(<redacted>)`;
    also `AuthorizationCode`, `DeviceCode`, `PkceVerifier` (lines 55-82).
  - `crates/forge_domain/src/auth/auth_token_response.rs:35-46` —
    `OAuthTokenResponse` redacts `access_token`/`refresh_token`/`id_token`.
  - `crates/forge_infra/src/auth/strategy.rs:638-662` — Codex device/token
    responses redact `device_auth_id`/`user_code`.

### Gaps / TODO

- **G1.1** — Tokens are stored in **plaintext** on disk. There is no OS keychain
  / Secret Service / DPAPI integration. 0o600 protects against other *users*
  but not against malware running *as the user*. TODO: optional keychain
  backend (`security-framework` on macOS, `secret-service` on Linux,
  `wincred` on Windows).
- **G1.2** — **Windows** has no equivalent of the 0o600 path (the
  `#[cfg(unix)]` guard means Windows files inherit default ACLs). TODO: set a
  restrictive Windows ACL or require keychain on Windows.
- **G1.3** — No integrity protection on the credential file (Tampering): a
  modified `base_url` is trusted. TODO: validate provider `base_url` against an
  allowlist of known hosts before use (note: a related exact-host Anthropic
  `base_url` check already exists for the Anthropic provider, commit
  `b3207ab01`).
- **G1.4** — No token-rotation / revocation-on-detection workflow if a leak is
  suspected. TODO: a `forge auth revoke` command + runbook entry.

---

## S2 — Prompt injection → tool / subprocess execution

The agent executes tools chosen from model output. Hostile content reaching the
model (a poisoned file, web page, or tool result) can attempt to coerce the
agent into running destructive tools (file write/delete, shell exec) or
exfiltrating data. Shell-capable tooling lives in `crates/forge_pheno_shell`,
and MCP subprocesses are spawned via `crates/forge_infra/src/mcp_client.rs`.

### STRIDE

- **Spoofing** — Injected text impersonating the user ("the user approved this").
- **Tampering** — Coercing edits to source, CI config, or the credential file.
- **Repudiation** — Actions taken on the user's behalf without clear logging.
- **Information disclosure** — Reading secrets and exfiltrating them through a
  tool call (e.g. a shell `curl`, or an MCP tool that egresses data).
- **Denial of service** — Forcing expensive/looping tool calls.
- **Elevation of privilege** — Escaping the working directory; running with the
  user's full shell privileges.

### Current mitigations (evidence-cited)

- **Per-operation permission policy.** Before any tool runs,
  `crates/forge_app/src/tool_registry.rs:64-91` converts the tool input to a
  policy operation (scoped to `cwd`) and calls
  `check_operation_permission(&operation)`. In restricted mode a denied
  permission blocks execution with a `permission_denied` error
  (`tool_registry.rs:142-155`) — the check happens *before* the call is
  dispatched.
- **CWD scoping.** Operations are bound to the current working directory
  (`tool_registry.rs:70`), constraining file operations.
- **Subprocess lifecycle hardening for MCP.** Stdio MCP children are spawned
  with `kill_on_drop(true)` and their stderr is piped + drained asynchronously
  to avoid deadlock (`crates/forge_infra/src/mcp_client.rs:148-172`).

### Gaps / TODO

- **G2.1** — There is **no allowlist of tool names** and no static
  "dangerous-tool" classification in the registry; trust relies entirely on the
  dynamic policy service. TODO: a default-deny allowlist for shell/network
  tools, surfaced in docs.
- **G2.2** — CWD scoping does not by itself prevent path traversal (`../`) or
  absolute paths from being passed to a tool. TODO: canonicalize + reject paths
  that escape the workspace root.
- **G2.3** — No data-egress guard: a permitted shell/MCP tool can read a secret
  and POST it out. TODO: optional network-egress confirmation for shell tools.
- **G2.4** — No structured audit trail of tool executions for after-the-fact
  review (Repudiation). TODO: append-only tool-execution log under `~/.forge/`.

---

## S3 — MCP server trust (untrusted MCP servers)

MCP servers are configured in `.mcp.json` as either **Stdio** (a local
subprocess) or **Http** (a network endpoint) — see
`crates/forge_domain/src/mcp.rs:19-93`. A Stdio server's `command`, `args`, and
`env` are taken directly from config and executed
(`crates/forge_infra/src/mcp_client.rs:148-172`). An MCP server is effectively
a **plugin with the same privileges as forge itself**.

### STRIDE

- **Spoofing** — A malicious MCP server advertises a trusted-looking tool name.
- **Tampering** — An HTTP MCP endpoint returns manipulated tool results that
  drive S2 prompt injection.
- **Repudiation** — Actions performed by an MCP tool are attributed to forge.
- **Information disclosure** — The subprocess inherits forge's environment
  (including any exported secrets) and runs with the user's permissions; an HTTP
  MCP server sees all arguments forge sends it.
- **Denial of service** — A hung or flooding MCP server stalls the agent.
- **Elevation of privilege** — A Stdio `command` is arbitrary code execution by
  design; a compromised `.mcp.json` is full RCE as the user.

### Current mitigations (evidence-cited)

- **Resilience isolation.** MCP calls go through a circuit breaker
  (name `"mcp_client"`, `crates/forge_infra/src/mcp_client.rs:71-104`) backed by
  `crates/forge_infra/src/resilience.rs` (Closed/Open/HalfOpen states,
  default failure threshold 5, reset timeout 30s) and a **bulkhead** semaphore
  capping concurrent MCP calls (default 16, immediate `BulkheadFullError` on
  saturation — `resilience.rs:195-242`). This bounds the DoS blast radius of a
  misbehaving server.
- **Process containment.** `kill_on_drop(true)` ensures Stdio servers die with
  the client; stderr is drained to prevent buffer-overflow deadlock
  (`mcp_client.rs:148-172`).
- **Per-tool permission gate.** MCP tools still pass through the S2 permission
  policy in `tool_registry.rs` before execution.

### Gaps / TODO

- **G3.1** — **No sandboxing.** Stdio MCP subprocesses run with full user
  privileges and inherit forge's environment. TODO: minimal env passthrough
  (don't forward credential env vars), and optional sandbox (e.g. `bwrap` /
  Seatbelt) for Stdio servers.
- **G3.2** — **No server allowlist / trust prompt.** Adding an MCP server is
  silent code-execution-on-startup. TODO: first-run trust confirmation per
  server with a recorded fingerprint of `command`+`args`.
- **G3.3** — **No TLS/cert pinning controls documented** for HTTP MCP servers.
  TODO: document and enforce `https://` for remote MCP, reject plaintext.
- **G3.4** — MCP tool name collisions with built-in tools are not prevented
  (Spoofing). TODO: namespace MCP tools and warn on shadowing.

---

## S4 — Telemetry egress (PostHog)

Anonymous usage events are sent to PostHog at
`https://us.i.posthog.com/capture/` (`crates/forge_tracker/src/collect/posthog.rs:76-77`).

### STRIDE

- **Spoofing** — N/A (forge is the sender; the write-only PostHog key is public
  by design).
- **Tampering** — A MITM could alter events (low impact; analytics only).
- **Repudiation** — N/A.
- **Information disclosure** — *Primary risk.* The dispatcher collects host
  metadata: `client_id`, OS / core count / **username**, **cwd**, **executable
  path**, and **CLI args** (`crates/forge_tracker/src/dispatch.rs:32-52`). cwd
  and CLI args can contain project names, paths, or even secrets passed on the
  command line.
- **Denial of service** — N/A for the user; an event flood is self-throttled.
- **Elevation of privilege** — N/A.

### Current mitigations (evidence-cited)

- **Opt-out / disabled by default in dev.** `can_track()` disables tracking for
  dev and `0.1.0` builds (`crates/forge_tracker/src/can_track.rs:8-19`).
- **Client-side rate limiting.** Max 1000 events/minute, fixed-window
  (`crates/forge_tracker/src/dispatch.rs:54-59`, `rate_limit.rs:3-45`) — bounds
  accidental egress volume.
- **Write-only key.** The PostHog secret is an ingestion key only
  (`dispatch.rs:20-23`), so the embedded value cannot read analytics back.

### Gaps / TODO

- **G4.1** — **CLI args and cwd are collected**; these can leak secrets or
  proprietary path/project names. TODO: scrub argv (drop values after
  `--token`/`--key`-style flags) and hash or drop cwd.
- **G4.2** — **No documented user-facing opt-out env var** for release builds.
  TODO: honor a `FORGE_TELEMETRY=0` / `DO_NOT_TRACK=1` env var and document it
  in the README + this model.
- **G4.3** — **Username is sent.** TODO: drop or hash `username`; it is PII.
- **G4.4** — No first-run telemetry-consent notice. TODO: print a one-time
  notice with the opt-out instructions.

---

## S5 — ZSH plugin / shell integration

forge integrates into the user's interactive shell. `forge.setup.zsh` adds a
managed block that `eval`s forge-generated zsh:
`eval "$(forge zsh plugin)"` and `eval "$(forge zsh theme)"`
(`shell-plugin/forge.setup.zsh:14,19`). The plugin captures terminal context
(preexec/precmd/OSC 133) and registers completion widgets
(`shell-plugin/lib/context.zsh`, `lib/completion.zsh:5-48`,
`lib/bindings.zsh`).

### STRIDE

- **Spoofing** — A modified `forge` on `$PATH` would have its output `eval`'d
  into the user's shell on every new session.
- **Tampering** — Editing the managed block in `.zshrc`, or any sourced
  `lib/*.zsh`, injects code into every shell.
- **Repudiation** — Shell-side actions are not logged by forge.
- **Information disclosure** — Context capture reads the user's command lines
  and terminal output; that data flows into forge prompts and could reach
  providers/telemetry.
- **Denial of service** — A slow preexec/precmd hook degrades every prompt.
- **Elevation of privilege** — `eval` of forge output is arbitrary code
  execution in the interactive shell's context on each startup.

### Current mitigations (evidence-cited)

- **Modular, reviewable plugin.** The plugin is split into auditable files
  (`shell-plugin/lib/{config,highlight,context,bindings,completion,dispatcher}.zsh`
  and `lib/actions/*`) rather than one opaque blob
  (`shell-plugin/forge.plugin.zsh:1-39`).
- **Widget-based completion** rather than executing arbitrary strings: the
  completion delegates to a Rust-built picker via `_forge_select_with_query()`
  (`shell-plugin/lib/completion.zsh:5-48`).
- **Managed, idempotent setup block** so the integration is contained and
  removable (`shell-plugin/forge.setup.zsh:1-21`).

### Gaps / TODO

- **G5.1** — `eval "$(forge zsh plugin)"` trusts `$PATH` resolution of `forge`
  on every shell start. TODO: document pinning to an absolute path, or ship a
  static, version-checked plugin file instead of `eval`-on-startup.
- **G5.2** — Captured terminal context may include secrets typed into the
  shell. TODO: document what context is captured and provide an opt-out /
  redaction for context capture.
- **G5.3** — No integrity check on the sourced `lib/*.zsh` files. TODO:
  ship a checksum manifest and verify on load.

---

## Summary of open gaps

| ID | Surface | Severity | Gap | Suggested fix |
|----|---------|----------|-----|---------------|
| G1.1 | Credentials | High | Plaintext token at rest | OS keychain backend |
| G1.2 | Credentials | High | No 0o600 equivalent on Windows | Restrictive ACL / keychain |
| G1.3 | Credentials | Med | `base_url` tampering trusted | Host allowlist validation |
| G1.4 | Credentials | Med | No revoke workflow | `forge auth revoke` + runbook |
| G2.1 | Tool exec | High | No tool allowlist | Default-deny shell/network tools |
| G2.2 | Tool exec | Med | Path traversal not blocked | Canonicalize + workspace-root check |
| G2.3 | Tool exec | Med | No egress guard | Optional network confirmation |
| G2.4 | Tool exec | Low | No audit trail | Append-only tool log |
| G3.1 | MCP | High | No sandbox / env isolation | Minimal env + optional sandbox |
| G3.2 | MCP | High | No trust prompt | First-run per-server confirmation |
| G3.3 | MCP | Med | Plaintext HTTP MCP allowed | Enforce HTTPS |
| G3.4 | MCP | Low | Tool name shadowing | Namespace + warn |
| G4.1 | Telemetry | Med | argv/cwd collected | Scrub argv, hash/drop cwd |
| G4.2 | Telemetry | Med | No documented opt-out env | Honor `FORGE_TELEMETRY=0`/`DO_NOT_TRACK` |
| G4.3 | Telemetry | Med | Username is PII | Drop/hash username |
| G4.4 | Telemetry | Low | No consent notice | One-time first-run notice |
| G5.1 | Shell | Med | `eval` trusts `$PATH` forge | Pin path / static plugin |
| G5.2 | Shell | Med | Context capture may hold secrets | Document + opt-out |
| G5.3 | Shell | Low | No plugin integrity check | Checksum manifest |

This document should be re-reviewed whenever a new external surface (provider,
MCP transport, telemetry sink, or shell hook) is added.
