#!/usr/bin/env zsh
# forge drift — command-hash hooks for overlap detection.
#
# Installs a preexec hook that SHA256-hashes the command line and forwards
# it to forge3d via the UDS control socket.  On precmd it polls for drift
# alerts and prints an informational line if an overlap is detected.
#
# Depends on:
#   forge3d  (daemon running on $FORGE3_SOCKET or default /tmp/forge3/daemon.sock)
#   sha256sum or shasum (POSIX — available on macOS)
#   zsh 5.8+
#
# Debug: set _FORGE_DRIFT_DEBUG=1 before sourcing to trace UDS traffic.
# Disable: export _FORGE_DRIFT_ENABLED=0  (per session)

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
: "${_FORGE_DRIFT_ENABLED:=1}"
: "${_FORGE_DRIFT_HASH_LEN:=16}"     # short-prefix length for display (0=full)
: "${_FORGE_DRIFT_TIMEOUT_MS:=200}"  # zsocket read timeout (approximate)
: "${_FORGE_DRIFT_AGENT_ID:=}"       # auto-derive from tmux pane if empty

# Default socket path: $FORGE3_SOCKET else XDG_RUNTIME_DIR else /tmp
if [[ -n "${FORGE3_SOCKET:-}" ]]; then
    _FORGE_DRIFT_SOCKET="$FORGE3_SOCKET"
elif [[ -n "${XDG_RUNTIME_DIR:-}" ]]; then
    _FORGE_DRIFT_SOCKET="${XDG_RUNTIME_DIR}/forge3/daemon.sock"
else
    _FORGE_DRIFT_SOCKET="/tmp/forge3/daemon.sock"
fi

# Agent ID: use $FORGE3_AGENT_ID, else tmux pane-title, else PID+HOST
if [[ -n "${FORGE3_AGENT_ID:-}" ]]; then
    _FORGE_DRIFT_AGENT_ID="$FORGE3_AGENT_ID"
elif [[ -n "${TMUX_PANE:-}" ]]; then
    _FORGE_DRIFT_AGENT_ID="tmux:${TMUX_PANE}"
else
    _FORGE_DRIFT_AGENT_ID="shell:${HOST:-localhost}:$$"
fi

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Compute SHA256 hex (16-char short-form or full).
# We accept either sha256sum (Linux, macOS-with-coreutils) or shasum -a 256.
_type() { whence -w "$1" 2>/dev/null; }
if _type sha256sum >/dev/null 2>&1; then
    _forge_drift_hash() {
        printf '%s' "$1" | sha256sum | cut -c1-$_FORGE_DRIFT_HASH_LEN
    }
elif _type shasum >/dev/null 2>&1; then
    _forge_drift_hash() {
        printf '%s' "$1" | shasum -a 256 | cut -c1-$_FORGE_DRIFT_HASH_LEN
    }
else
    # Fallback: simple POSIX md5 using /sbin/md5 on macOS, md5sum elsewhere
    if _type md5 >/dev/null 2>&1; then
        _forge_drift_hash() { printf '%s' "$1" | md5 | cut -c1-$_FORGE_DRIFT_HASH_LEN; }
    elif _type md5sum >/dev/null 2>&1; then
        _forge_drift_hash() { printf '%s' "$1" | md5sum | cut -c1-$_FORGE_DRIFT_HASH_LEN; }
    else
        # Bleeding-edge minimal: just LC_ALL=C awk (worse collisions, but works everywhere)
        _forge_drift_hash() { printf '%s' "$1" | LC_ALL=C awk '{s=0; for(i=1;i<=length($0);i++){c=index("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",substr($0,i,1));if(c)s=(s*31+c)%999999};printf "%06x\n",s}'; }
    fi
fi

# ---------------------------------------------------------------------------
# UDS framing — JSON-RPC 2.0 with 4-byte length prefix
# ---------------------------------------------------------------------------
_forge_drift_send() {
    local method="$1" params="$2"
    local json
    json=$(printf '{"jsonrpc":"2.0","method":"%s","params":%s,"id":%d}' \
        "$method" "$params" "$(( RANDOM % 9999 + 1 ))")

    # 4-byte big-endian length prefix
    local len=${#json}
    local prefix
    prefix=$(printf '\\x%02x\\x%02x\\x%02x\\x%02x' \
        $(( (len >> 24) & 0xFF )) \
        $(( (len >> 16) & 0xFF )) \
        $(( (len >>  8) & 0xFF )) \
        $((   len        & 0xFF )))

    # Try non-blocking connect to UDS socket
    if [[ ! -S "$_FORGE_DRIFT_SOCKET" ]]; then
        [[ -n "${_FORGE_DRIFT_DEBUG:-}" ]] && print "[forge-drift] socket not found: $_FORGE_DRIFT_SOCKET" >&2
        return 1
    fi

    # Use zsh's ztcp for UDS (zsh 5.3+)
    local fd
    if ! zsocket -d "$_FORGE_DRIFT_SOCKET" 2>/dev/null; then
        [[ -n "${_FORGE_DRIFT_DEBUG:-}" ]] && print "[forge-drift] zsocket failed (daemon down?)" >&2
        return 1
    fi
    fd=$REPLY

    # Send prefix + JSON
    print -nu "$fd" "$prefix$json"

    # Read response with short timeout (blocking but likely immediate)
    local resp=""
    local maxloop=5
    while (( maxloop-- )); do
        local chunk=""
        if sysread -t 0.1 -i "$fd" chunk 2>/dev/null; then
            resp+="$chunk"
            # Stop after we've read a complete JSON object
            if [[ "$resp" == *$'\n' ]]; then
                break
            fi
        else
            break
        fi
    done

    exec {fd}>&-

    [[ -n "$resp" ]] && print -r -- "$resp"
    return 0
}

# ---------------------------------------------------------------------------
# Preexec hook — compute command hash and send to forge3d
# ---------------------------------------------------------------------------
function _forge_drift_preexec() {
    [[ "$_FORGE_DRIFT_ENABLED" != "1" ]] && return
    local cmdline="$1"

    # Skip empty and very short commands (cd, ls, etc.)
    local stripped="${cmdline## #}"
    if [[ -z "$stripped" || ${#stripped} -lt 4 ]]; then
        return
    fi

    local hash_val
    hash_val=$(_forge_drift_hash "$stripped")

    # Store for potential display in precmd
    typeset -g _FORGE_DRIFT_LAST_HASH="$hash_val"
    typeset -g _FORGE_DRIFT_LAST_CMD="$stripped"

    [[ -n "${_FORGE_DRIFT_DEBUG:-}" ]] && print "[forge-drift] observing: $hash_val <- $stripped" >&2

    # Send to daemon (non-blocking — we don't wait for the response)
    _forge_drift_send "drift.observe" \
        "{\"agent_id\":\"${_FORGE_DRIFT_AGENT_ID}\",\"prompt\":\"${stripped//\"/\\\"}\",\"hash\":\"${hash_val}\"}" \
        >/dev/null 2>&1 &
}

# ---------------------------------------------------------------------------
# Precmd hook — check for drift alerts from daemon
# ---------------------------------------------------------------------------
function _forge_drift_precmd() {
    [[ "$_FORGE_DRIFT_ENABLED" != "1" ]] && return

    # Quick poll for alerts using a lightweight "ack" method
    local resp
    resp=$(_forge_drift_send "drift.check" \
        "{\"agent_id\":\"${_FORGE_DRIFT_AGENT_ID}\",\"limit\":3}" 2>/dev/null)

    if [[ -z "$resp" ]]; then
        return
    fi

    # Minimal JSON parsing: look for an "overlap" or "alert" key in the response
    if [[ "$resp" == *'"overlap"'* || "$resp" == *'"OverlapAlert"'* ]]; then
        # Extract alert count and top similarity
        local sim
        sim=$(print -r -- "$resp" | LC_ALL=C sed -n 's/.*"similarity":[[:space:]]*\([0-9.]*\).*/\1/p' | head -1)
        local other_id
        other_id=$(print -r -- "$resp" | LC_ALL=C sed -n 's/.*"other_agent_id":[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)

        # Use newline-safe print to avoid mangling the prompt
        if [[ -n "$other_id" && -n "$sim" ]]; then
            printf '\n\033[33m⚠\033[0m forge-drift: similar cmd in %s (sim=%.2f). Run \033[33mforge drift show\033[0m\n' \
                "$other_id" "$sim"
        else
            printf '\n\033[33m⚠\033[0m forge-drift: overlap detected. Run \033[33mforge drift show\033[0m\n'
        fi
    fi
}

# ---------------------------------------------------------------------------
# Registration — prepend to existing hooks
# ---------------------------------------------------------------------------
if [[ "$_FORGE_DRIFT_ENABLED" == "1" ]]; then
    # preexec: drift goes BEFORE context (want hash before ring-buffer push)
    preexec_functions=(_forge_drift_preexec "${preexec_functions[@]}")
    # precmd: drift goes LAST (after context has captured exit code)
    precmd_functions+=("_forge_drift_precmd")
fi
