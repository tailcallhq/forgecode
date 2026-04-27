#!/usr/bin/env zsh

# Custom completion widget that handles both :commands and @ completion

function forge-completion() {
    local current_word="${LBUFFER##* }"
    
    # Handle @ completion (files and directories)
    if [[ "$current_word" =~ ^@.*$ ]]; then
        local filter_text="${current_word#@}"
        local selected
        
        # Use Rust's built-in file picker
        if [[ -n "$filter_text" ]]; then
            selected=$(_forge_select file --query "$filter_text")
        else
            selected=$(_forge_select file)
        fi
        
        if [[ -n "$selected" ]]; then
            selected="@[${selected}]"
            LBUFFER="${LBUFFER%$current_word}"
            BUFFER="${LBUFFER}${selected}${RBUFFER}"
            CURSOR=$((${#LBUFFER} + ${#selected}))
        fi
        
        zle reset-prompt
        return 0
    fi
    
    # Handle :command completion (supports letters, numbers, hyphens, underscores)
    if [[ "${LBUFFER}" =~ "^:([a-zA-Z][a-zA-Z0-9_-]*)?$" ]]; then
        # Extract the text after the colon for filtering
        local filter_text="${LBUFFER#:}"
        
        # Use Rust's built-in command picker
        local selected
        if [[ -n "$filter_text" ]]; then
            selected=$(_forge_select command --query "$filter_text")
        else
            selected=$(_forge_select command)
        fi
        
        if [[ -n "$selected" ]]; then
            # Replace the current buffer with the selected command
            BUFFER=":$selected "
            CURSOR=${#BUFFER}
        fi
        
        zle reset-prompt
        return 0
    fi
    
    # Fall back to default completion
    zle expand-or-complete
}
