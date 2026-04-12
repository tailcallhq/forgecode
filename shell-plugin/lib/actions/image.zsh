#!/usr/bin/env zsh

# Image action handlers

# Action handler: Paste image from clipboard
function _forge_action_paste_image() {
    # Call forge paste-image to save the image and get the @[path] string
    local paste_output
    paste_output=$(_forge_exec paste-image 2>/dev/null)
    
    if [[ $? -eq 0 && -n "$paste_output" ]]; then
        # Append to existing text or replace
        local input_text="$1"
        if [[ -n "$input_text" ]]; then
            BUFFER="$input_text $paste_output"
        elif [[ -n "$BUFFER" && "$WIDGET" != "forge-accept-line" ]]; then
             # If called from a keybinding and buffer is not empty
             BUFFER="$BUFFER $paste_output"
        else
            BUFFER="$paste_output"
        fi
        CURSOR=${#BUFFER}
        # Only call zle if it exists
        [[ -n "$WIDGET" ]] && zle reset-prompt
    else
        echo
        _forge_log error "No image found in clipboard or failed to save."
        _forge_reset
    fi
}
