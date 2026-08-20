# ForgeCode Runbook

> Status: Living document. Owner: ForgeCode maintainers.
> Last reviewed: 2026-06-28 (Phase P4 overhaul).
> Audience: end users and maintainers diagnosing a misbehaving local install.

ForgeCode is a local tool, so "operations" means **diagnosing one machine**.
Each entry below has: symptoms, likely cause, diagnosis, and fix. State paths
referenced live under `~/.forge/`.

Key local artifacts:

- `~/.forge/forge.db` — SQLite conversation store (daemon).
- `~/.forge/.forge.db.sock` — Unix domain socket for `forge_dbd`
  (`crates/forge_dbd/src/main.rs:15-23`).
- `~/.forge/.forge.db` — legacy client conversation store (read side of the
  split-DB union).
- `~/.forge/.forge.writes.db` — client conversation write target (split-DB
  default; reads union the legacy `.forge.db` via the `conversations_all`
  TEMP VIEW).
- `~/.forge/.credentials.json` — provider credentials (mode 0o600).
- `~/.forge/.mcp-credentials.json` — MCP OAuth state (mode 0o600).
- `.mcp.json` — MCP server configuration.

---

## 1. Database locked / conversation writes failing

**Symptoms:** conversation history not persisting; errors mentioning the
database being busy/locked; daemon log shows write failures.

**Likely cause:** Two `forge_dbd` instances bound to the same DB, a stale lock
from a crashed daemon, or a WAL that needs checkpointing.

**Diagnosis:**

1. Check whether more than one daemon is running:
   `ps aux | grep forge_dbd`
2. Check the socket exists and is fresh: `ls -la ~/.forge/.forge.db.sock`
3. Confirm reachability with a health probe (see §2).

**Fix:**

1. Stop extra daemon instances (leave one).
2. The daemon removes a stale socket before binding
   (`crates/forge_dbd/src/server.rs:75-78`); if a stale socket persists after a
   hard crash, remove it manually: `rm ~/.forge/.forge.db.sock`, then restart
   the daemon.
3. If WAL growth is the issue, the daemon supports `Request::CheckpointWal`
   (`crates/forge_dbd/src/protocol.rs`); trigger a checkpoint, or restart the
   daemon to flush.
4. **Last resort** — back up `~/.forge/forge.db`, then move it aside and let the
   daemon recreate it. *Never* delete it without a backup; this is conversation
   history. The store is best-effort, so loss degrades but does not break forge.

---

## 2. Daemon down / unreachable

**Symptoms:** persistence features unavailable; forge still functions (graceful
degradation) but history isn't saved; health probe fails.

**Likely cause:** Daemon not started, crashed (SIGTERM/SIGINT path), socket
missing, or `~/.forge/` permissions wrong.

**Diagnosis:**

1. Is it running? `ps aux | grep forge_dbd`
2. Does the socket exist? `ls -la ~/.forge/.forge.db.sock`
3. **Health probe**: the daemon answers `Request::Ping` inline with
   `Response::Health` (`crates/forge_dbd/src/server.rs:168-241`). A successful
   round-trip means the accept loop and writer are alive.
4. Check `~/.forge/` exists and is user-owned (the daemon creates the parent
   dir if missing — `server.rs:81-83`).

**Fix:**

1. Restart the daemon. It binds the Unix listener, removing any stale socket
   first (`server.rs:75-85`).
2. If it exits immediately, check for a permissions problem on `~/.forge/`
   (must be writable by the user) or a port/socket conflict.
3. Daemon shutdown is graceful on SIGTERM/SIGINT and flushes a final batch
   before exit (`server.rs:101-120`, `247-289`); an unclean kill may leave a
   stale socket — remove it (§1) and restart.
4. Remember: a *deliberately* disabled daemon is fine — forge degrades
   gracefully. Only an *expected-up-but-unreachable* daemon is an incident.

---

## 3. Authentication expired / 401s from provider

**Symptoms:** provider calls fail with auth errors; OAuth token rejected;
"please re-authenticate" prompts.

**Likely cause:** Expired OAuth access token whose refresh failed, a corrupted
`~/.forge/.credentials.json`, or a tampered/incorrect `base_url`.

**Diagnosis:**

1. Confirm the credential file exists and has mode 0o600:
   `ls -la ~/.forge/.credentials.json` (enforced by
   `crates/forge_repo/src/provider/provider_repo.rs:609-616`).
2. **Do not** cat the file into a shared log — it contains live tokens. Secret
   values are redacted in forge's own logs by design
   (`crates/forge_domain/src/auth/auth_token_response.rs:35-46`), so forge logs
   are safe to share; the raw file is not.
3. Verify the configured `base_url` points at the real provider host (tamper
   check — see threat-model G1.3).

**Fix:**

1. Re-run the provider login/OAuth flow to mint fresh tokens.
2. If the file is corrupt, back it up and re-authenticate to regenerate it.
3. If a token leak is suspected, **revoke it provider-side** and re-auth; rotate
   any API key. (A dedicated `forge auth revoke` workflow is a tracked gap —
   threat-model G1.4.)
4. On Windows note that 0o600 is not applied (threat-model G1.2); ensure the
   file is not on a shared/synced path.

---

## 4. Provider 429 / rate limiting and circuit-breaker behavior

**Symptoms:** requests slow then suddenly **fail fast**; logs reference the
`"mcp_client"` breaker being open, or repeated 429/5xx; throughput drops.

**Likely cause:** The provider (or an MCP server) is rate-limiting or
overloaded, so the resilience layer is shedding load to protect the session.

**Diagnosis & expected behavior:**

1. **Retry first.** Retryable statuses
   `[429, 500, 502, 503, 504, 408, 522, 524, 520, 529]` and overload errors are
   retried with exponential backoff
   (`crates/forge_config/src/retry.rs:6-26`,
   `crates/forge_repo/src/provider/retry.rs:9-37`). Transient 429s should
   self-heal.
2. **Circuit breaker.** After 5 consecutive failures the breaker opens and calls
   fail *immediately* for ~30s, then a half-open probe tests recovery
   (`crates/forge_infra/src/resilience.rs:47-132`). Fast failures right after a
   burst of errors are **expected** — the breaker is doing its job, not a bug.
3. **Bulkhead.** `BulkheadFullError` means too many concurrent MCP calls
   (default cap 16, `resilience.rs:195-242`); the system is shedding, not
   broken.

**Fix:**

1. Wait out the breaker's reset window (~30s) — it self-recovers via the
   half-open probe on the next success.
2. Reduce concurrency / request rate if you are hitting the bulkhead or
   sustained 429s; check your provider plan's rate limits.
3. For a flaky MCP server, disable it in `.mcp.json` (`disable: true`) to stop
   tripping its breaker.
4. If the breaker opens during *normal* (low-rate) use, that is an SLO
   regression — capture logs and file an issue; investigate the upstream
   provider before assuming a forge bug.

---

## 5. MCP server misbehaving / hanging

**Symptoms:** agent stalls on tool calls; a specific MCP tool never returns;
breaker for `"mcp_client"` keeps opening.

**Likely cause:** A slow, hung, or flooding MCP server (Stdio subprocess or HTTP
endpoint).

**Diagnosis:**

1. Identify the server in `.mcp.json`.
2. For Stdio servers, check the child process; forge spawns them with
   `kill_on_drop(true)` and drains stderr
   (`crates/forge_infra/src/mcp_client.rs:148-172`).
3. Confirm whether the bulkhead/breaker is shedding (§4).

**Fix:**

1. Set `disable: true` for the offending server in `.mcp.json` and retry.
2. Restart forge to respawn a clean Stdio child.
3. Treat any *untrusted* MCP server with caution — it runs with your privileges
   (threat-model S3).

---

## 6. Shell integration broken (ZSH)

**Symptoms:** new shells error on startup; completion widget missing; prompt
slow.

**Likely cause:** `eval "$(forge zsh plugin)"` resolved a wrong/missing `forge`
on `$PATH`, or a sourced `lib/*.zsh` is broken
(`shell-plugin/forge.setup.zsh:14,19`).

**Fix:**

1. Confirm `which forge` resolves to the intended binary.
2. Temporarily comment out the managed block in `.zshrc` to isolate whether
   forge is the cause; reopen a shell.
3. Re-run forge's shell setup to regenerate the managed block.
4. If a precmd/preexec hook is slow, investigate context capture
   (`shell-plugin/lib/context.zsh`) — see threat-model S5/G5.2 for the opt-out
   discussion.

---

## Escalation

If a fix above does not resolve the issue, gather: forge version,
`ps aux | grep forge`, the daemon health-probe result, and **redacted** forge
logs (secrets are already redacted by forge's Debug impls — never paste the raw
credential files), then file an issue or follow the postmortem template in
`docs/operations/postmortem-template.md` for anything user-impacting and novel.
