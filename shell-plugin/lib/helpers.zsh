#!/usr/bin/env zsh

# Core utility functions for forge plugin

# Lazy loader for commands cache
# Loads the commands list only when first needed, avoiding startup cost
function _forge_get_commands() {
    if [[ -z "$_FORGE_COMMANDS" ]]; then
        _FORGE_COMMANDS="$(CLICOLOR_FORCE=0 $_FORGE_BIN list commands --porcelain 2>/dev/null)"
    fi
    echo "$_FORGE_COMMANDS"
}

# Private fzf function with common options for consistent UX
function _forge_fzf() {
    fzf --reverse --exact --cycle --select-1 --height 80% --no-scrollbar --ansi --color="header:bold" "$@"
}

# Helper function to execute forge commands consistently
# This ensures proper handling of special characters and consistent output
function _forge_exec() {
    local agent_id="${_FORGE_ACTIVE_AGENT:-forge}"
    local -a cmd
    cmd=($_FORGE_BIN --agent "$agent_id")

    # Expose terminal context arrays as US-separated (\x1F) env vars so that
    # the Rust TerminalContextService can read them via get_env_var.
    # ASCII Unit Separator (\x1F) is used instead of `:` because commands
    # can legitimately contain colons (URLs, port mappings, paths, etc.).
    # Use `local -x` so the variables are exported only to the child forge
    # process and do not leak into the caller's shell environment.
    if [[ "$_FORGE_TERM" == "true" && ${#_FORGE_TERM_COMMANDS} -gt 0 ]]; then
        # Join the ring-buffer arrays with the ASCII Unit Separator (\x1F).
        # We use IFS-based joining ("${arr[*]}") rather than ${(j.SEP.)arr} because
        # zsh does NOT expand $'...' ANSI-C escapes inside parameter expansion flags.
        local _old_ifs="$IFS" _sep=$'\x1f'
        IFS="$_sep"
        local -x _FORGE_TERM_COMMANDS="${_FORGE_TERM_COMMANDS[*]}"
        local -x _FORGE_TERM_EXIT_CODES="${_FORGE_TERM_EXIT_CODES[*]}"
        local -x _FORGE_TERM_TIMESTAMPS="${_FORGE_TERM_TIMESTAMPS[*]}"
        IFS="$_old_ifs"
    fi

    cmd+=("$@")
    [[ -n "$_FORGE_SESSION_MODEL" ]] && local -x FORGE_SESSION__MODEL_ID="$_FORGE_SESSION_MODEL"
    [[ -n "$_FORGE_SESSION_PROVIDER" ]] && local -x FORGE_SESSION__PROVIDER_ID="$_FORGE_SESSION_PROVIDER"
    [[ -n "$_FORGE_SESSION_REASONING_EFFORT" ]] && local -x FORGE_REASONING__EFFORT="$_FORGE_SESSION_REASONING_EFFORT"
    "${cmd[@]}"
}

# Like _forge_exec but connects stdin/stdout to /dev/tty so that interactive
# prompts (rustyline, fzf, etc.) work correctly when forge is launched as a
# child of a ZLE widget. ZLE owns the terminal and replaces the process's
# stdin/stdout with its own pipes, so without this redirect any readline
# library would see a non-tty stdin and return EOF immediately.
# Do NOT use inside $(...) command substitutions - use _forge_exec instead.
function _forge_exec_interactive() {
    local agent_id="${_FORGE_ACTIVE_AGENT:-forge}"
    local -a cmd
    cmd=($_FORGE_BIN --agent "$agent_id")

    # Expose terminal context arrays as US-separated (\x1F) env vars so that
    # the Rust TerminalContextService can read them via get_env_var.
    # ASCII Unit Separator (\x1F) is used instead of `:` because commands
    # can legitimately contain colons (URLs, port mappings, paths, etc.).
    # Use `local -x` so the variables are exported only for the duration of
    # this function call (i.e. inherited by the child forge process) and do
    # not leak into the caller's shell environment.
    if [[ "$_FORGE_TERM" == "true" && ${#_FORGE_TERM_COMMANDS} -gt 0 ]]; then
        local _old_ifs="$IFS" _sep=$'\x1f'
        IFS="$_sep"
        local -x _FORGE_TERM_COMMANDS="${_FORGE_TERM_COMMANDS[*]}"
        local -x _FORGE_TERM_EXIT_CODES="${_FORGE_TERM_EXIT_CODES[*]}"
        local -x _FORGE_TERM_TIMESTAMPS="${_FORGE_TERM_TIMESTAMPS[*]}"
        IFS="$_old_ifs"
    fi

    cmd+=("$@")
    [[ -n "$_FORGE_SESSION_MODEL" ]] && local -x FORGE_SESSION__MODEL_ID="$_FORGE_SESSION_MODEL"
    [[ -n "$_FORGE_SESSION_PROVIDER" ]] && local -x FORGE_SESSION__PROVIDER_ID="$_FORGE_SESSION_PROVIDER"
    [[ -n "$_FORGE_SESSION_REASONING_EFFORT" ]] && local -x FORGE_REASONING__EFFORT="$_FORGE_SESSION_REASONING_EFFORT"
    "${cmd[@]}" </dev/tty >/dev/tty
}

function _forge_reset() {
  # Clear buffer and reset cursor position
  BUFFER=""
  CURSOR=0
  # Force widget redraw and prompt reset
  zle -I
  zle reset-prompt
}

# Helper function to find the index of a value in a list (1-based)
# Returns the index if found, 1 otherwise
# Usage: _forge_find_index <output> <value_to_find> [field_number] [field_number2] [value_to_find2]
# field_number: which porcelain column to compare (1-based, using multi-space delimiter)
# field_number2/value_to_find2: optional second column+value for compound matching
# Note: This function expects porcelain output WITH headers and skips the header line
function _forge_find_index() {
    local output="$1"
    local value_to_find="$2"
    local field_number="${3:-1}"
    local field_number2="${4:-}"
    local value_to_find2="${5:-}"

    local index=1
    local line_num=0
    while IFS= read -r line; do
        ((line_num++))
        # Skip the header line (first line)
        if [[ $line_num -eq 1 ]]; then
            continue
        fi
        
        local field_value=$(echo "$line" | awk -F '  +' "{print \$$field_number}")
        if [[ "$field_value" == "$value_to_find" ]]; then
            if [[ -n "$field_number2" && -n "$value_to_find2" ]]; then
                local field_value2=$(echo "$line" | awk -F '  +' "{print \$$field_number2}")
                if [[ "$field_value2" == "$value_to_find2" ]]; then
                    echo "$index"
                    return 0
                fi
            else
                echo "$index"
                return 0
            fi
        fi
        ((index++))
    done <<< "$output"

    echo "1"
    return 0
}

# Helper function to print messages with consistent formatting based on log level
# Usage: _forge_log <level> <message>
# Levels: error, info, success, warning, debug
# Color scheme matches crates/forge_main/src/title_display.rs
function _forge_log() {
    local level="$1"
    local message="$2"
    local timestamp="\033[90m[$(date '+%H:%M:%S')]\033[0m"
    
    case "$level" in
        error)
            # Category::Error - Red ⏺
            echo "\033[31m⏺\033[0m ${timestamp} \033[31m${message}\033[0m"
            ;;
        info)
            # Category::Info - White ⏺
            echo "\033[37m⏺\033[0m ${timestamp} \033[37m${message}\033[0m"
            ;;
        success)
            # Category::Action/Completion - Yellow ⏺
            echo "\033[33m⏺\033[0m ${timestamp} \033[37m${message}\033[0m"
            ;;
        warning)
            # Category::Warning - Bright yellow ⚠️
            echo "\033[93m⚠️\033[0m ${timestamp} \033[93m${message}\033[0m"
            ;;
        debug)
            # Category::Debug - Cyan ⏺ with dimmed text
            echo "\033[36m⏺\033[0m ${timestamp} \033[90m${message}\033[0m"
            ;;
        *)
            echo "${message}"
            ;;
    esac
}

# Helper function to check if a workspace is indexed
# Usage: _forge_is_workspace_indexed <workspace_path>
# Returns: 0 if workspace is indexed, 1 otherwise
function _forge_is_workspace_indexed() {
    local workspace_path="$1"
    $_FORGE_BIN workspace info "$workspace_path" >/dev/null 2>&1
    return $?
}

# Start background sync job for current workspace if not already running
# Uses canonical path hash to identify workspace
function _forge_start_background_sync() {
    # Check if sync is enabled (default to true if not set)
    local sync_enabled="${FORGE_SYNC_ENABLED:-true}"
    if [[ "$sync_enabled" != "true" ]]; then
        return 0
    fi

    # Get canonical workspace path
    local workspace_path=$(pwd -P)

    # Check if workspace is indexed before attempting sync
    {
        # Run sync once in background
        # Close all output streams immediately to prevent any flashing
        # Redirect stdin to /dev/null to prevent hanging if sync tries to read input
        exec >/dev/null 2>&1 </dev/null
        setopt NO_NOTIFY NO_MONITOR
        if ! _forge_is_workspace_indexed "$workspace_path"; then
            return 0
        fi
        # Should fail if sync-init or sync --init has not been performed even once
        $_FORGE_BIN workspace sync "$workspace_path"
    } &!
}

# Start background update check if not already running
# Mirrors the background sync pattern to silently check for and apply updates
function _forge_start_background_update() {
    {
        # Run update check in background
        # Close all output streams immediately to prevent any flashing
        # Redirect stdin to /dev/null to prevent hanging
        exec >/dev/null 2>&1 </dev/null
        setopt NO_NOTIFY NO_MONITOR
        $_FORGE_BIN update --no-confirm
    } &!
}

