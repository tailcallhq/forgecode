#!/usr/bin/env zsh

# Key bindings and widget registration for forge plugin

# Register ZLE widgets
zle -N forge-accept-line
zle -N forge-completion

# Custom bracketed-paste handler that wraps dropped file paths in @[] syntax
# and fixes syntax highlighting after paste.
#
# Path detection and wrapping is delegated to `forge zsh format` (Rust) so
# that all parsing logic lives in one well-tested place.
function forge-bracketed-paste() {
    # Call the built-in bracketed-paste widget first
    zle .$WIDGET "$@"
    
    # Only auto-wrap when the line is a forge command (starts with ':').
    # This avoids mangling paths pasted into normal shell commands like
    # 'vim /some/path' or 'cat /some/path'.
    if [[ "$BUFFER" == :* ]]; then
        local formatted=$("$_FORGE_BIN" zsh format --buffer "$BUFFER")
        if [[ -n "$formatted" && "$formatted" != "$BUFFER" ]]; then
            BUFFER="$formatted"
            CURSOR=${#BUFFER}
        fi
    fi
    
    # Explicitly redisplay the buffer to ensure paste content is visible
    # This is critical for large or multiline pastes
    zle redisplay
    
    # Reset the prompt to trigger syntax highlighting refresh
    # The redisplay before reset-prompt ensures the buffer is fully rendered
    zle reset-prompt
}

# Tab widget that was bound to ^I before Forge installed its own binding.
# Empty when that binding could not be read. Kept across later applies so
# zsh-vi-mode's keymap rebuild does not replace it with forge-completion.
typeset -g _FORGE_TAB_WIDGET

function _forge_capture_tab_widget() {
    # The first successful capture wins. Later applies run after ^I is
    # already forge-completion.
    [[ -n "${_FORGE_TAB_WIDGET:-}" ]] && return 0

    local binding widget
    binding="$(bindkey '^I' 2>/dev/null)" || return 0
    [[ "$binding" == *" "* ]] || return 0

    # bindkey prints: "^I" <widget>
    widget="${binding#* }"
    widget="${widget## }"
    if [[ -z "$widget" || "$widget" == "forge-completion" ]]; then
        return 0
    fi

    _FORGE_TAB_WIDGET="$widget"
}

# Re-applied after zsh-vi-mode's `zvm_init` precmd hook, which rebuilds the
# main/viins/vicmd keymaps and otherwise silently clobbers these bindings.
function _forge_apply_keybindings() {
    _forge_capture_tab_widget
    zle -N bracketed-paste forge-bracketed-paste
    bindkey '^M' forge-accept-line
    bindkey '^J' forge-accept-line
    bindkey '^I' forge-completion
}

_forge_apply_keybindings

# Harmless no-op when zsh-vi-mode (jeffreytse/zsh-vi-mode) isn't loaded.
typeset -ga zvm_after_init_commands
zvm_after_init_commands+=('_forge_apply_keybindings')
