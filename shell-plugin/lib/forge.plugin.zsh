#!/usr/bin/env zsh

# forge plugin - modular loader
# Sources all required modules in dependency order

# Guard against double-sourcing
[[ -n "$_FORGE_PLUGIN_LOADED" ]] && return 0

# ---------------------------------------------------------------------------
# Core configuration (must be first - provides variables used by other modules)
# ---------------------------------------------------------------------------
source "${0:A:h}/config.zsh"

# ---------------------------------------------------------------------------
# Helpers (provides utility functions like _forge_exec, _forge_log)
# ---------------------------------------------------------------------------
source "${0:A:h}/helpers.zsh"

# ---------------------------------------------------------------------------
# Terminal context capture (preexec/precmd hooks, OSC 133)
# ---------------------------------------------------------------------------
source "${0:A:h}/context.zsh"

# ---------------------------------------------------------------------------
# Main dispatcher and widget registration
# ---------------------------------------------------------------------------
source "${0:A:h}/dispatcher.zsh"

# ---------------------------------------------------------------------------
# Key bindings and widget registration
# ---------------------------------------------------------------------------
source "${0:A:h}/bindings.zsh"

# ---------------------------------------------------------------------------
# Syntax highlighting
# ---------------------------------------------------------------------------
source "${0:A:h}/highlight.zsh"

# ---------------------------------------------------------------------------
# Completion widget
# ---------------------------------------------------------------------------
source "${0:A:h}/completion.zsh"

# ---------------------------------------------------------------------------
# Action handlers (must be loaded last as they define handlers for dispatcher)
# ---------------------------------------------------------------------------
source "${0:A:h}/actions/core.zsh"
source "${0:A:h}/actions/config_actions.zsh"
source "${0:A:h}/actions/conversation.zsh"
source "${0:A:h}/actions/git.zsh"
source "${0:A:h}/actions/auth.zsh"
source "${0:A:h}/actions/editor.zsh"
source "${0:A:h}/actions/provider.zsh"

# Mark plugin as loaded
export _FORGE_PLUGIN_LOADED=1
