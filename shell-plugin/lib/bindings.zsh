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

# Integrate with zsh-autosuggestions.
#
# zsh-autosuggestions wraps widgets listed in ZSH_AUTOSUGGEST_CLEAR_WIDGETS,
# ZSH_AUTOSUGGEST_ACCEPT_WIDGETS, etc. so it can clear POSTDISPLAY (the gray
# inline suggestion) before the widget runs. The wrapping happens once, when
# autosuggestions loads. If Forge's plugin is sourced *after* autosuggestions
# (the Oh-My-Zsh default when users follow `forge zsh setup`), our
# forge-accept-line widget is defined too late to be wrapped, and the inline
# suggestion lingers on the screen after the user presses Enter.
#
# Fix: append forge-accept-line to ZSH_AUTOSUGGEST_CLEAR_WIDGETS and re-run
# the bind pass so the wrapper is installed. Safe no-op when autosuggestions
# is not loaded or when the widget is already registered.
if typeset -p ZSH_AUTOSUGGEST_CLEAR_WIDGETS >/dev/null 2>&1; then
    if [[ ${ZSH_AUTOSUGGEST_CLEAR_WIDGETS[(Ie)forge-accept-line]} -eq 0 ]]; then
        ZSH_AUTOSUGGEST_CLEAR_WIDGETS+=(forge-accept-line)
        if typeset -f _zsh_autosuggest_bind_widgets >/dev/null 2>&1; then
            _zsh_autosuggest_bind_widgets
        fi
    fi
fi
