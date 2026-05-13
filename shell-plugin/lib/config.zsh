#!/usr/bin/env zsh

# Configuration variables for forge plugin
# Using typeset to keep variables local to plugin scope and prevent public exposure

typeset -h _FORGE_BIN="${FORGE_BIN:-forge}"
typeset -h _FORGE_CONVERSATION_PATTERN=":"
typeset -h _FORGE_MAX_COMMIT_DIFF="${FORGE_MAX_COMMIT_DIFF:-100000}"

typeset -h _FORGE_COMMANDS=""

# Hidden variables to be used only via the ForgeCLI
typeset -h _FORGE_CONVERSATION_ID
typeset -h _FORGE_ACTIVE_AGENT

# Previous conversation ID for :conversation - (like cd -)
typeset -h _FORGE_PREVIOUS_CONVERSATION_ID

# Session-scoped model and provider overrides (set via :model / :m).
# When non-empty, these are passed as --model / --provider to every forge
# invocation for the lifetime of the current shell session.
typeset -h _FORGE_SESSION_MODEL
typeset -h _FORGE_SESSION_PROVIDER

# Session-scoped reasoning effort override (set via :reasoning-effort / :re).
# When non-empty, exported as FORGE_REASONING__EFFORT for every forge invocation.
typeset -h _FORGE_SESSION_REASONING_EFFORT

# Terminal context capture settings
# Master switch for terminal context capture (preexec/precmd hooks)
typeset -h _FORGE_TERM="${FORGE_TERM:-true}"
# Maximum number of commands to keep in the ring buffer (metadata: cmd + exit code)
typeset -h _FORGE_TERM_MAX_COMMANDS="${FORGE_TERM_MAX_COMMANDS:-5}"
# OSC 133 semantic prompt marker emission: "auto", "on", or "off"
typeset -h _FORGE_TERM_OSC133="${FORGE_TERM_OSC133:-auto}"
# Ring buffer arrays for context capture
typeset -ha _FORGE_TERM_COMMANDS=()
typeset -ha _FORGE_TERM_EXIT_CODES=()
typeset -ha _FORGE_TERM_TIMESTAMPS=()
