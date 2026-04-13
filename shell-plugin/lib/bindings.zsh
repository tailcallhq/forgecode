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

# Register the bracketed paste widget to fix highlighting on paste
zle -N bracketed-paste forge-bracketed-paste

# Bind Enter to our custom accept-line that transforms :commands
bindkey '^M' forge-accept-line
bindkey '^J' forge-accept-line
# Update the Tab binding to use the new completion widget
bindkey '^I' forge-completion  # Tab for both @ and :command completion

# Fix: zsh-vi-mode plugin compatibility
# When zsh-vi-mode (jeffreytse/zsh-vi-mode) is active, Enter in vi-command mode
# does not trigger forge's colon commands. Fix by also binding Enter in vicmd.
# Detects both zsh-vi-mode plugin ($ZVM_MODE) and native vi mode (bindkey -v).
if [[ -n "$ZVM_MODE" ]] || bindkey -lL main 2>/dev/null | grep -q "bindkey -A viins main\|bindkey -A vicmd main"; then
    bindkey -M vicmd '^M' forge-accept-line
    bindkey -M vicmd '^J' forge-accept-line
    # Also bind Tab in vicmd for @ and :command completion
    bindkey -M vicmd '^I' forge-completion
fi
