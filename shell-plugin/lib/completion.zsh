#!/usr/bin/env zsh

# Custom completion widget that handles both :commands and @ completion

function _forge_select_file_completion() {
    local filter_text="$1"
    local output_file exit_status
    typeset -g _FORGE_COMPLETION_SELECTED=""

    output_file=$(mktemp -t forge-file-select-output.XXXXXX) || return 1

    zle -I
    if [[ -n "$filter_text" ]]; then
        CLICOLOR_FORCE=0 "$_FORGE_BIN" select file --query "$filter_text" </dev/tty >"$output_file" 2>/dev/tty
    else
        CLICOLOR_FORCE=0 "$_FORGE_BIN" select file </dev/tty >"$output_file" 2>/dev/tty
    fi
    exit_status=$?

    if [[ $exit_status -eq 0 && -s "$output_file" ]]; then
        IFS= read -r _FORGE_COMPLETION_SELECTED < "$output_file"
    fi

    rm -f "$output_file"
    return $exit_status
}

function forge-completion() {
    local current_word="${LBUFFER##* }"
    
    # Handle @ completion (files and directories)
    if [[ "$current_word" =~ ^@.*$ ]]; then
        local filter_text="${current_word#@}"
        local selected
        
        # Use Rust's built-in file picker with preview
        _forge_select_file_completion "$filter_text"
        selected="$_FORGE_COMPLETION_SELECTED"
        
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
            selected=$(CLICOLOR_FORCE=0 $_FORGE_BIN select command --query "$filter_text" </dev/tty 2>/dev/tty)
        else
            selected=$(CLICOLOR_FORCE=0 $_FORGE_BIN select command </dev/tty 2>/dev/tty)
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
