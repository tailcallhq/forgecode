#!/usr/bin/env zsh

#:# ZSH glue for Ghostty terminal — auto-detect, IPC bridge, keybindings
#:#
#:# Load with: `autoload -U forge_ghostty_detect && forge_ghostty_detect`
#:#
#:# Functions provided:
#:#   forge_ghostty_detect   — probe for Ghostty + control socket
#:#   forge_ghostty_call     — low-level bridge to `forge ghostty` IPC
#:#   forge_ghostty_title    — set window title
#:#   forge_ghostty_progress — set progress indicator
#:#   forge_ghostty_reload   — reload Ghostty config from disk
#:#   forge_ghostty_open     — open URL in browser
#:#   forge_ghostty_bind     — register default keybindings (^G^t, ^G^r, ^G^p)

# Guard against double-sourcing
[[ -n "${_FORGE_GHOSTTY_ZSH_LOADED:-}" ]] && return 0
typeset -g _FORGE_GHOSTTY_ZSH_LOADED=1

emulate -L zsh
setopt local_options pipe_fail no_unset

# ---------------------------------------------------------------------------
# Private config
# ---------------------------------------------------------------------------

# Binary the bridge calls. Defaults to whatever is on PATH, but allows
# override for testing (e.g. `FORGE_GHOSTTY_BIN=/tmp/fake-forge`).
typeset -g _FORGE_GHOSTTY_BIN="${FORGE_GHOSTTY_BIN:-forge}"
# Cached control socket path (empty when not found).
typeset -g _FORGE_GHOSTTY_SOCKET=""

# ---------------------------------------------------------------------------
# Detection helpers
# ---------------------------------------------------------------------------

# Resolve the Ghostty control socket path.
# Echoes the first existing socket from the well-known locations.
# Locations checked, in priority order:
#   1. $GHOSTTY_CONTROL_SOCKET (if set and is a socket)
#   2. $XDG_RUNTIME_DIR/ghostty/control.sock
#   3. /tmp/ghostty-control.sock
# Returns 0 and prints the path on success, 1 and prints nothing on failure.
function _forge_ghostty_socket_path() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    if [[ -n "${GHOSTTY_CONTROL_SOCKET:-}" && -S "$GHOSTTY_CONTROL_SOCKET" ]]; then
        print -- "$GHOSTTY_CONTROL_SOCKET"
        return 0
    fi

    local xdg="${XDG_RUNTIME_DIR:-}"
    if [[ -n "$xdg" && -S "$xdg/ghostty/control.sock" ]]; then
        print -- "$xdg/ghostty/control.sock"
        return 0
    fi
    if [[ -n "$xdg" && -S "$xdg/ghostty.sock" ]]; then
        print -- "$xdg/ghostty.sock"
        return 0
    fi

    if [[ -S /tmp/ghostty-control.sock ]]; then
        print -- /tmp/ghostty-control.sock
        return 0
    fi

    return 1
}

# Detect whether we are running inside Ghostty AND the control socket is
# reachable. Sets the global `FORGE_GHOSTTY_AVAILABLE=1` on success, 0 otherwise.
# Returns 0 if available, 1 if not. Caches the result and the resolved socket.
function forge_ghostty_detect() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local detected=0

    # Strategy 1: $TERM_PROGRAM is set to "ghostty" (most reliable signal;
    # Ghostty exports this on launch).
    if [[ "${TERM_PROGRAM:-}" == "ghostty" ]]; then
        detected=1
    fi

    # Strategy 2: the parent process tree contains a process whose comm
    # contains "ghostty". Covers nested shells, sshd passthrough, etc.
    if (( detected == 0 )) && [[ -r "/proc/${PPID:-0}/status" ]]; then
        local grandpid
        grandpid=$(awk '/^PPid:/ {print $2; exit}' "/proc/${PPID}/status" 2>/dev/null) || grandpid=""
        if [[ -n "$grandpid" ]] && command -v ps >/dev/null 2>&1; then
            local comm
            comm=$(ps -o comm= -p "$grandpid" 2>/dev/null) || comm=""
            if [[ "$comm" == *ghostty* ]]; then
                detected=1
            fi
        fi
    fi

    # Strategy 3: control socket exists and is reachable. Even if we can't
    # identify the parent, the socket being present is a strong signal.
    if (( detected == 0 )); then
        local sock
        sock=$(_forge_ghostty_socket_path 2>/dev/null) || sock=""
        if [[ -n "$sock" ]]; then
            detected=1
        fi
    fi

    if (( detected == 1 )); then
        _FORGE_GHOSTTY_SOCKET=$(_forge_ghostty_socket_path 2>/dev/null) || _FORGE_GHOSTTY_SOCKET=""
        typeset -g FORGE_GHOSTTY_AVAILABLE=1
        return 0
    fi

    _FORGE_GHOSTTY_SOCKET=""
    typeset -g FORGE_GHOSTTY_AVAILABLE=0
    return 1
}

# ---------------------------------------------------------------------------
# IPC bridge
# ---------------------------------------------------------------------------

# Low-level bridge to `forge ghostty --action <verb> --args '<json>'`.
# Stderr is forwarded to the user's terminal (or /dev/tty inside a ZLE
# widget); stdout is captured and echoed to the caller.
#
# Usage: forge_ghostty_call <verb> [<json-or-flags>...]
# Returns 0 on success, 1 on any failure.
function forge_ghostty_call() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local verb="${1:-}"
    if [[ -z "$verb" ]]; then
        print -u2 -- "forge_ghostty_call: missing action verb"
        return 1
    fi
    shift

    if (( ! $+commands[forge] )); then
        print -u2 -- "forge_ghostty_call: 'forge' binary not found on PATH"
        return 1
    fi

    # Defer the expensive detection until the user actually calls us; reuse
    # a previous positive result, but re-probe on a previous negative so the
    # user can `source` us in a new Ghostty tab and have it pick up.
    if [[ "${FORGE_GHOSTTY_AVAILABLE:-}" != "1" ]]; then
        if ! forge_ghostty_detect; then
            print -u2 -- "forge_ghostty_call: Ghostty not available in this terminal"
            return 1
        fi
    fi

    # In a ZLE widget, stdout is connected to the line buffer. Redirect
    # the bridge's stdout to /dev/tty so we don't corrupt the buffer;
    # callers that need the JSON can call us outside a ZLE widget.
    local -a cmd
    cmd=("$_FORGE_GHOSTTY_BIN" ghostty --action "$verb")
    if (( $# > 0 )); then
        cmd+=(--args "$*")
    else
        cmd+=(--args "{}")
    fi

    if [[ -n "${WIDGET:-}" ]]; then
        "$_FORGE_GHOSTTY_BIN" "${cmd[@]:1}" 2>/dev/tty >/dev/tty
        return $?
    fi

    "$_FORGE_GHOSTTY_BIN" "${cmd[@]:1}" 2>/dev/tty
    return $?
}

# ---------------------------------------------------------------------------
# High-level wrappers
# ---------------------------------------------------------------------------

# Set the Ghostty window title.
# Usage: forge_ghostty_title "My Title"
function forge_ghostty_title() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local title="$*"
    if [[ -z "$title" ]]; then
        print -u2 -- "forge_ghostty_title: title argument required"
        return 1
    fi
    forge_ghostty_call set_window_title "$title"
}

# Set the Ghostty progress indicator.
# Usage: forge_ghostty_progress <state> [<value>]
#   state: default | normal | error | indeterminate
#   value: 0-100 (required for normal/error, ignored for default/indeterminate)
function forge_ghostty_progress() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local state="${1:-}"
    local value="${2:-}"

    case "$state" in
        default|normal|error|indeterminate) ;;
        "")
            print -u2 -- "forge_ghostty_progress: state argument required (default|normal|error|indeterminate)"
            return 1
            ;;
        *)
            print -u2 -- "forge_ghostty_progress: invalid state '$state'"
            return 1
            ;;
    esac

    # Validate value when state needs it.
    if [[ "$state" == "normal" || "$state" == "error" ]]; then
        if [[ -z "$value" ]]; then
            print -u2 -- "forge_ghostty_progress: state '$state' requires a value (0-100)"
            return 1
        fi
        if ! [[ "$value" == <-> ]] || (( value < 0 || value > 100 )); then
            print -u2 -- "forge_ghostty_progress: value must be an integer 0-100 (got '$value')"
            return 1
        fi
    fi

    local payload
    if [[ -n "$value" ]]; then
        payload="{\"state\":\"$state\",\"value\":$value}"
    else
        payload="{\"state\":\"$state\"}"
    fi

    forge_ghostty_call set_progress "$payload"
}

# Reload Ghostty configuration from disk.
function forge_ghostty_reload() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    forge_ghostty_call reload_config
}

# Open a URL in the user's default browser via Ghostty.
# Usage: forge_ghostty_open "https://example.com"
function forge_ghostty_open() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local url="${1:-}"
    if [[ -z "$url" ]]; then
        print -u2 -- "forge_ghostty_open: url argument required"
        return 1
    fi
    forge_ghostty_call open_url "$url"
}

# ---------------------------------------------------------------------------
# ZLE widgets + keybinding registration
# ---------------------------------------------------------------------------

# ZLE widget: prompt for a new window title (bound to ^G^t).
function forge-ghostty-title-widget() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local new_title=""
    if [[ -r /dev/tty ]]; then
        vared -p "ghostty title> " -c new_title </dev/tty 2>/dev/tty
    else
        vared -p "ghostty title> " -c new_title
    fi
    if [[ -n "$new_title" ]]; then
        forge_ghostty_title "$new_title"
    fi
    zle reset-prompt
}

# ZLE widget: reload Ghostty config from disk (bound to ^G^r).
function forge-ghostty-reload-widget() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    forge_ghostty_reload
    zle reset-prompt
}

# ZLE widget: interactive progress state select (bound to ^G^p).
# Prompts for one of: default (d), normal (n, defaults to 25%), error (e),
# indeterminate (i). Any other key cancels.
function forge-ghostty-progress-widget() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local key
    if [[ -r /dev/tty ]]; then
        print -n "ghostty progress [d=default n=normal e=error i=indeterminate]: " >/dev/tty
        read -k key </dev/tty 2>/dev/tty
    else
        print -n "ghostty progress [d=default n=normal e=error i=indeterminate]: "
        read -k key
    fi
    print -- "" </dev/tty 2>/dev/tty

    case "$key" in
        d|D) forge_ghostty_progress default ;;
        n|N) forge_ghostty_progress normal 25 ;;
        e|E) forge_ghostty_progress error 50 ;;
        i|I) forge_ghostty_progress indeterminate ;;
        *)   zle reset-prompt; return 0 ;;
    esac
    zle reset-prompt
}

# Register the default keybindings on the main keymap.
# No-ops gracefully if zle is not loaded (e.g. non-interactive shell, sourced
# from a script). Safe to call multiple times — `zle -N` re-registers cleanly.
#
# Usage: forge_ghostty_bind
function forge_ghostty_bind() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    # `zle` is a shell builtin only in interactive shells. If it's not
    # defined, we're in a non-interactive context (e.g. `zsh -c`); skip.
    if ! typeset -f zle >/dev/null 2>&1; then
        return 0
    fi

    zle -N forge-ghostty-title-widget
    zle -N forge-ghostty-reload-widget
    zle -N forge-ghostty-progress-widget

    bindkey '^G^t' forge-ghostty-title-widget
    bindkey '^G^r' forge-ghostty-reload-widget
    bindkey '^G^p' forge-ghostty-progress-widget
}
