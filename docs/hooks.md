# External Hook System

Forge supports external hook scripts that intercept and modify tool calls before execution. This document covers the architecture, security model, cross-platform behavior, and trade-offs of the hook system.

## Overview

The hook system allows users to place executable scripts in `~/.forge/hooks/<event>.d/` directories. When Forge processes a tool call, it pipes the call details to each hook via stdin as JSON, reads a JSON response from stdout, and uses the result to allow (optionally with modifications), or deny the tool call.

Currently the only supported event is `toolcall-start`, triggered before every tool call execution.

## Architecture

### Component Layout

```
forge_domain                    forge_app                       forge_main
-----------                     ---------                       ----------
ToolCallInterceptor (trait)     ExternalHookInterceptor          forge hook list
Hook (lifecycle container)     PreparedHook (executable)        forge hook trust <path>
CachedHook (in-memory content) TrustStore (SHA-256 + trust.json) forge hook delete <path>
LifecycleEvent (enum)          load_and_verify_hooks()          discover_hooks()
EventHandle (trait)            discover_hooks()
```

### Lifecycle

```
Startup
  |
  v
discover_hooks("toolcall-start")
  |  Scans ~/.forge/hooks/toolcall-start.d/ for executable files
  |  Returns sorted PathBuf list
  v
load_and_verify_hooks()
  |  Loads TrustStore from ~/.forge/hooks/trust.json
  |  For each hook: check trust status (SHA-256)
  |  Trusted -> read content into CachedHook
  |  Untrusted/Tampered -> skip with warning
  v
ExternalHookInterceptor::new(cached_hooks)
  |  For each CachedHook: prepare_executable()
  |  Linux: memfd_create -> write -> fchmod -> seal -> /proc/self/fd/<n>
  |  Non-Linux: compute SHA-256 hash of source file for runtime verification
  |  Returns PreparedHook (reusable for entire session)
  v
[Session runs, tool calls happen]
  |
  v
intercept(tool_call)
  |  For each PreparedHook:
  |    Linux: spawn() memfd path directly
  |    Non-Linux: spawn() -> re-read file -> verify hash -> exec original path
  |    deny -> return error (abort tool call)
  |    allow (with hookSpecificOutput) -> update tool_input for next hook
  |    allow (no modification) -> pass through
  |    failure/timeout/invalid JSON -> degrade to allow
  v
Tool call executes with (possibly modified) arguments
```

### Key Types

| Type | Location | Purpose |
|------|----------|---------|
| `ToolCallInterceptor` | `forge_domain::hook` | Async trait for intercepting tool calls |
| `EventHandle<T>` | `forge_domain::hook` | Async trait for observing lifecycle events |
| `Hook` | `forge_domain::hook` | Container holding handlers for all 6 lifecycle events + one interceptor |
| `CachedHook` | `forge_domain::cached_hook` | In-memory hook content (source path + bytes), created at startup |
| `PreparedHook` | `forge_app::hooks::external` | Pre-compiled executable (memfd on Linux) or hash-verified source path (non-Linux) |
| `Executable` | `forge_app::hooks::external` | Enum: `Memfd` (Linux) or `Source` with expected hash (non-Linux) |
| `TrustStore` | `forge_app::hooks::trust` | JSON-backed store of SHA-256 hashes at `~/.forge/hooks/trust.json` |
| `HookTrustStatus` | `forge_app::hooks::trust` | Enum: `Trusted`, `Untrusted`, `Tampered`, `Missing` |

### Hook Composition

Hooks compose via `Hook::zip()`, which chains handlers and interceptors sequentially. The `CombinedInterceptor` runs the first interceptor, then passes the (possibly modified) tool call to the second. External hooks are further chained inside `ExternalHookInterceptor::intercept()` as a pipeline -- each hook's modified output feeds into the next.

## Communication Protocol

### Input (Forge -> hook stdin)

```json
{"tool_name": "shell", "tool_input": {"command": "git status"}}
```

### Output (hook stdout -> Forge)

**Allow without modification:**
```json
{"decision": "allow"}
```

**Allow with modification:**
```json
{"decision": "allow", "hookSpecificOutput": {"tool_input": {"command": "rtk git status"}}}
```

**Deny:**
```json
{"decision": "deny", "reason": "blocked by policy"}
```

### Pipeline Behavior

Multiple hooks in the same event directory execute in **alphabetical filename order**. Each hook receives the (possibly modified) `tool_input` from the previous hook. If any hook returns `deny`, the pipeline short-circuits and the tool call is rejected.

### Failure Handling

| Failure Mode | Behavior |
|-------------|----------|
| Hook exits non-zero | Degrade to `allow` (pass-through) |
| Hook output is not valid JSON | Degrade to `allow` |
| Hook exceeds timeout (default 30s) | Degrade to `allow`, process killed |
| Unknown `decision` value | Degrade to `allow` |
| Hook fails to spawn | Degrade to `allow` |

This fail-open design prioritizes availability: a broken hook should not block the user's workflow.

## Security Model

### Threat Model

The hook system defends against:

1. **TOCTOU (Time-of-check to time-of-use)**: An attacker modifies a hook script between integrity verification and execution.
2. **Path traversal**: A symlink or `..` component in a hook path escapes `~/.forge/hooks/`.
3. **Supply chain tampering**: A hook script is silently modified after the user trusted it.

### Integrity Verification (SHA-256 Trust Store)

At startup, `load_and_verify_hooks()` checks each discovered hook against `~/.forge/hooks/trust.json`:

| Status | Condition | Action |
|--------|-----------|--------|
| `Trusted` | SHA-256 matches stored hash | Load content into `CachedHook` |
| `Untrusted` | No entry in trust store | Skip, print guidance |
| `Tampered` | SHA-256 mismatch | Print DANGER warning, remove from trust store, do NOT load |
| `Missing` | File disappeared | Skip with warning |

Trust is managed via CLI:
- `forge hook trust <path>` -- compute hash, store in `trust.json`
- `forge hook delete <path>` -- remove file + trust entry
- `forge hook list` -- show all hooks with trust status

The trust store file itself is written with `0o444` permissions (read-only) as a speed bump, and uses atomic write (temp file + rename) to prevent corruption.

### TOCTOU Mitigation

**Linux (memfd -- zero TOCTOU):**

Hook content is loaded into memory at startup and executed from an anonymous memory file descriptor (memfd):

1. `memfd_create()` creates an in-memory fd
2. Content is written via `libc::write()`
3. `libc::fchmod(fd, 0o700)` marks it executable
4. `seal_memfd()` applies `F_SEAL_SEAL | F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_WRITE` -- content is immutable
5. Hook executes via `/proc/self/fd/<n>`

The `MFD_CLOEXEC` flag is intentionally **not** set so that shebang interpreters (e.g. `#!/usr/bin/env python3`) can re-open the script path after `execve`. The memfd guard (`OwnedFd`) is held for the entire session lifetime, ensuring the fd remains valid.

**Non-Linux (hash verification -- detect TOCTOU):**

macOS and Windows do not have `memfd_create` (macOS lacks procfs entirely, so even `shm_open` cannot be used for execution). Instead, the original source file is spawned directly after verifying its integrity:

1. At construction time, the SHA-256 hash of the on-disk file is computed and stored in `Executable::Source { expected_hash }`
2. Before each `spawn()`, the file is re-read and its hash is compared against the expected value
3. If the hash matches, the original source path is spawned directly
4. If the hash mismatches, the spawn fails and the hook is treated as `allow` (fail-open)

This approach eliminates the need for temporary files entirely. The TOCTOU window is reduced to the microsecond gap between hash verification and `execve`, which is comparable to the previous tempfile approach. The key advantage is that no long-lived temporary file exists on disk -- the only file involved is the user's own hook script in `~/.forge/hooks/`, which is protected by home directory permissions.

**Why not memfd on macOS?**

macOS has no `memfd_create` system call. While it has `shm_open` (POSIX shared memory), macOS lacks procfs, so there is no `/proc/self/fd/<n>` path to pass to `execve`. Without a file path that the kernel can execute, memfd-style execution is not possible on macOS.

### Path Traversal Prevention

`validate_hook_path()` canonicalizes both the hook path and the base directory (`~/.forge/hooks/`), then verifies the canonical hook path starts with the canonical base. This resolves symlinks and normalizes `..` components.

`validate_hook_path_for_delete()` handles the case where the file no longer exists (cannot canonicalize), falling back to lexical normalization of path components.

`relative_hook_path()` has a fallback for symlinked HOME directories: if stripping the non-canonical base fails, it tries the canonical base.

### Timeout Isolation

Each hook execution has a configurable timeout (default 30 seconds). On timeout, the child process is killed (via `kill_on_drop(true)`) and the result degrades to `allow`. This prevents hooks from blocking the session indefinitely.

## Cross-Platform Behavior

### Hook Discovery

| Platform | Discovery Method |
|----------|-----------------|
| Unix (Linux, macOS) | Check executable (`x`) permission bit on file |
| Non-Unix (Windows) | Include all regular files; OS determines executability via file association |

### Hook Execution

| Platform | Execution Method | Details |
|----------|-----------------|---------|
| Linux | memfd (`/proc/self/fd/<n>`) | Zero disk I/O, sealed against modification |
| macOS / other Unix | Direct source path | Hash verified before each spawn; no temp file created |
| Windows | Direct source path | Hash verified before each spawn; `.py` works via file association, `.sh` unlikely |

### Shebang Support

| Platform | Shebang Support | Notes |
|----------|----------------|-------|
| Linux | Yes | Kernel processes shebang via `execve`; `MFD_CLOEXEC` not set so interpreter can re-open the fd |
| macOS | Yes | Direct source file on disk, kernel processes shebang normally |
| Windows | No | Windows uses file extension association instead; shebang lines are ignored as comments |

### Python Hook Compatibility

Python hooks (`*.py`) work on all platforms:
- **Linux**: memfd execution; `__file__` will be `/proc/self/fd/<n>` (cosmetic only)
- **macOS**: direct source file execution; shebang `#!/usr/bin/env python3` works normally
- **Windows**: direct source file with `.py` extension; Windows file association launches Python; shebang ignored

## CLI Commands

```
forge hook list              # List all hooks with trust status
forge hook trust <path>      # Trust a hook (compute + store SHA-256)
forge hook delete <path>     # Delete a hook file + remove trust entry
```

The `<path>` argument accepts:
- Relative path from `~/.forge/hooks/` (e.g. `toolcall-start.d/01-hook.sh`)
- Bare filename if unique (e.g. `01-hook.sh`)

## Example Hooks

### 1. Deny dangerous shell commands

Block shell commands that match dangerous patterns:

```bash
#!/bin/bash
# ~/.forge/hooks/toolcall-start.d/01-block-dangerous.sh
# Deny shell commands containing rm -rf, force push, or drop database

read input
tool_name=$(echo "$input" | jq -r '.tool_name')

if [ "$tool_name" = "shell" ]; then
    cmd=$(echo "$input" | jq -r '.tool_input.command')
    case "$cmd" in
        *"rm -rf"*|*"rm -r /"*|*"force push"*|*"DROP DATABASE"*|*"DROP TABLE"*)
            echo "{\"decision\":\"deny\",\"reason\":\"blocked dangerous command: $cmd\"}"
            ;;
        *)
            echo '{"decision":"allow"}'
            ;;
    esac
else
    echo '{"decision":"allow"}'
fi
```

### 2. Dry-run validation

Run a dry-run of the command to validate arguments before allowing execution:

```bash
#!/bin/bash
# ~/.forge/hooks/toolcall-start.d/02-dry-run-validate.sh
# Dry-run commands that support it (ffmpeg, rsync, etc.) to catch bad arguments

read input
tool_name=$(echo "$input" | jq -r '.tool_name')

if [ "$tool_name" != "shell" ]; then
    echo '{"decision":"allow"}'
    exit 0
fi

cmd=$(echo "$input" | jq -r '.tool_input.command')

case "$cmd" in
    ffmpeg*\ -y*|ffmpeg*\ --yes*)
        # ffmpeg: run without output files to validate arguments
        dry_cmd=$(echo "$cmd" | sed 's/ffmpeg/ffmpeg -nostdin -hide_banner/')
        if ! $dry_cmd -f lavfi -i nullsrc=s=256x256:d=0.1 -f null - 2>/dev/null; then
            echo "{\"decision\":\"deny\",\"reason\":\"ffmpeg dry-run failed: invalid arguments\"}"
            exit 0
        fi
        ;;
    rsync\ --dry-run*|rsync\ -n*)
        # Already a dry-run, allow as-is
        ;;
    rsync\ *)
        # rsync: run with --dry-run to validate
        if ! $(echo "$cmd" | sed 's/rsync/rsync --dry-run/') 2>/dev/null; then
            echo "{\"decision\":\"deny\",\"reason\":\"rsync dry-run failed: invalid arguments\"}"
            exit 0
        fi
        ;;
esac

echo '{"decision":"allow"}'
```

### 3. Require command prefix

Prefix all shell commands with a wrapper (e.g., `rtk` for audit logging):

```bash
#!/bin/bash
# ~/.forge/hooks/toolcall-start.d/03-prefix-shell.sh
# Wrap all shell commands with 'rtk' for audit logging

read input
tool_name=$(echo "$input" | jq -r '.tool_name')

if [ "$tool_name" = "shell" ]; then
    original=$(echo "$input" | jq -r '.tool_input.command')
    echo "{\"decision\":\"allow\",\"hookSpecificOutput\":{\"tool_input\":{\"command\":\"rtk $original\"}}}"
else
    echo '{"decision":"allow"}'
fi
```

### 4. Deny reading sensitive files

Prevent the LLM from reading files matching sensitive paths:

```bash
#!/bin/bash
# ~/.forge/hooks/toolcall-start.d/04-protect-secrets.sh
# Deny file reads from sensitive directories

read input
tool_name=$(echo "$input" | jq -r '.tool_name')

if [ "$tool_name" = "read" ]; then
    path=$(echo "$input" | jq -r '.tool_input.file_path')
    case "$path" in
        *"/.env"*|*"/.ssh/"*|*"/.gnupg/"*|*"/.aws/"*|*"/credentials"*|*"/secrets/"*)
            echo "{\"decision\":\"deny\",\"reason\":\"access denied: $path contains sensitive data\"}"
            ;;
        *)
            echo '{"decision":"allow"}'
            ;;
    esac
else
    echo '{"decision":"allow"}'
fi
```

### 5. Node version check

Deny shell commands if the system Node.js version is below 22:

```bash
#!/bin/bash
# ~/.forge/hooks/toolcall-start.d/05-node-version-check.sh
# Deny shell commands if Node.js < 22 is installed

read input
tool_name=$(echo "$input" | jq -r '.tool_name')

if [ "$tool_name" != "shell" ]; then
    echo '{"decision":"allow"}'
    exit 0
fi

node_version=$(node --version 2>/dev/null) || {
    echo '{"decision":"allow"}'
    exit 0
}

# node --version outputs e.g. "v22.1.0"
major=$(echo "$node_version" | sed 's/^v//;s/\..*//')

if [ "$major" -lt 22 ]; then
    echo "{\"decision\":\"deny\",\"reason\":\"Node.js $node_version is below minimum required v22\"}"
else
    echo '{"decision":"allow"}'
fi
```

### Setting up a hook

After placing a hook script:

```bash
chmod +x ~/.forge/hooks/toolcall-start.d/<hook-name>.sh
forge hook trust toolcall-start.d/<hook-name>.sh
```

Multiple hooks execute in alphabetical filename order. Use numeric prefixes to control ordering (e.g., `01-`, `02-`, `03-`).

## Architecture Trade-offs

### Advantages

1. **Language-agnostic**: Any executable can be a hook -- shell, Python, compiled binaries, etc. Users are not locked into a specific language or runtime.
2. **Fail-open by default**: Hook failures (crash, timeout, invalid output) degrade to `allow`, ensuring a broken hook does not block the user's session.
3. **Zero runtime disk I/O on Linux**: memfd execution means hook content lives entirely in memory after startup. No TOCTOU window.
4. **Simple protocol**: stdin/stdout JSON is easy to produce and consume from any language. No shared libraries, FFI, or socket protocols required.
5. **Composable pipeline**: Multiple hooks chain naturally via alphabetical ordering. Each hook can independently allow, modify, or deny.
6. **Integrity verification**: SHA-256 trust store detects tampering at startup. Tampered hooks are refused with clear remediation instructions.

### Limitations

1. **Process spawn overhead**: Each hook invocation forks a new process. For high-frequency tool calls, this adds latency (typically 10-50ms per hook per call). Not suitable for real-time interception at scale.
2. **No streaming support**: The protocol is request-response over stdin/stdout. Hooks cannot stream partial results or participate in streaming tool output.
3. **Fail-open design**: By design, failures degrade to `allow`. This is a deliberate availability-over-security trade-off. A hook that should enforce a policy (e.g., "block all `rm -rf`") can be bypassed by crashing or timing out.
4. **Trust store is not a security boundary**: An attacker with write access to `~/.forge/hooks/trust.json` can replace hashes. The `0o444` permission is a speed bump, not a guarantee. The trust model assumes the user's home directory is not compromised.
5. **No hook-specific environment isolation**: Hooks inherit the full process environment. There is no sandboxing, chroot, or capability restriction. A malicious hook has the same privileges as the Forge process.
6. **Windows `.sh` support is impractical**: While `.py` hooks work via file association, `.sh` hooks require a Unix-like shell which is not standard on Windows. The system primarily targets Unix-like platforms.
7. **Single event type**: Currently only `toolcall-start` is supported. There is no mechanism for `toolcall-end`, `request`, or `response` hooks via the external script interface (though the internal `Hook` struct supports all lifecycle events).
8. **No hook versioning or dependency management**: Hooks are independent scripts with no versioning, dependency declaration, or compatibility checking. Upgrading Forge could silently break hooks if the JSON protocol changes.

### Design Decisions

| Decision | Rationale |
|----------|-----------|
| stdin/stdout JSON | Simplest cross-language protocol; no sockets, FFI, or shared memory |
| Fail-open on error | Availability > security for a developer tool; broken hooks should not block work |
| Alphabetical ordering | Deterministic, predictable, no configuration needed |
| memfd on Linux | Eliminates TOCTOU entirely; content is immutable after seal |
| Hash verification on non-Linux | Detects TOCTOU at spawn time without creating temp files; no long-lived temp file on disk |
| Trust store (not interactive prompts) | Non-interactive startup; trust is managed explicitly via CLI |
| `kill_on_drop(true)` | Ensures child processes are cleaned up even if the caller forgets to wait |
| `CachedHook` in `forge_domain` | Decouples domain flow from platform-specific execution details |
