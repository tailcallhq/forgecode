#!/usr/bin/env zsh

# Documentation in [README.md](./README.md)

# Modular forge plugin - sources all required modules

# Configuration variables
source "${0:A:h}/lib/config.zsh"

# Syntax highlighting
source "${0:A:h}/lib/highlight.zsh"

# Core utilities (includes logging)
source "${0:A:h}/lib/helpers.zsh"

# Terminal context capture (preexec/precmd hooks, OSC 133)
source "${0:A:h}/lib/context.zsh"

# Completion widget
source "${0:A:h}/lib/completion.zsh"

# Action handlers
source "${0:A:h}/lib/actions/core.zsh"
source "${0:A:h}/lib/actions/config.zsh"
source "${0:A:h}/lib/actions/conversation.zsh"
source "${0:A:h}/lib/actions/git.zsh"
source "${0:A:h}/lib/actions/auth.zsh"
source "${0:A:h}/lib/actions/editor.zsh"
source "${0:A:h}/lib/actions/provider.zsh"
source "${0:A:h}/lib/actions/doctor.zsh"
source "${0:A:h}/lib/actions/keyboard.zsh"

# Main dispatcher and widget registration
source "${0:A:h}/lib/dispatcher.zsh"

# Key bindings and widget registration
source "${0:A:h}/lib/bindings.zsh"
