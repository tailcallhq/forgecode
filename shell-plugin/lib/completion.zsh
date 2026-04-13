#!/usr/bin/env zsh

# Custom completion widget that handles both :commands and @ completion

function forge-completion() {
    local current_word="${LBUFFER##* }"
    
    # Handle @ completion (files and directories)
    if [[ "$current_word" =~ ^@.*$ ]]; then
        local filter_text="${current_word#@}"
        local selected
        local fzf_args=(
            --preview="if [ -d {} ]; then ls -la --color=always {} 2>/dev/null || ls -la {}; else $_FORGE_CAT_CMD {}; fi"
            $_FORGE_PREVIEW_WINDOW
        )
        
        local file_list=$(${FORGE_BIN:-forge} list files --porcelain)
        if [[ -n "$filter_text" ]]; then
            selected=$(echo "$file_list" | _forge_fzf --query "$filter_text" "${fzf_args[@]}")
        else
            selected=$(echo "$file_list" | _forge_fzf "${fzf_args[@]}")
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
        
        # Lazily load the commands list
        local commands_list=$(_forge_get_commands)
        if [[ -n "$commands_list" ]]; then
            # Use fzf for interactive selection with prefilled filter
            local selected
            if [[ -n "$filter_text" ]]; then
                selected=$(echo "$commands_list" | _forge_fzf --header-lines=1 --delimiter="$_FORGE_DELIMITER" --nth=1 --query "$filter_text" --prompt="Command ❯ ")
            else
                selected=$(echo "$commands_list" | _forge_fzf --header-lines=1 --delimiter="$_FORGE_DELIMITER" --nth=1 --prompt="Command ❯ ")
            fi
            
            if [[ -n "$selected" ]]; then
                # Extract just the command name (first word before any description)
                local command_name="${selected%% *}"
                # Replace the current buffer with the selected command
                BUFFER=":$command_name "
                CURSOR=${#BUFFER}
            fi
        fi
        
        zle reset-prompt
        return 0
    fi
    
    # Fall back to default completion
    zle expand-or-complete
}
