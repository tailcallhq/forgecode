#!/usr/bin/env zsh

# Core action handlers for basic forge operations

# Action handler: Start a new conversation
function _forge_action_new() {
    local input_text="$1"
    
    # Clear conversation and save as previous (like cd -)
    _forge_clear_conversation
    _FORGE_ACTIVE_AGENT="forge"
    
    echo
    
    # If input_text is provided, send it to the new conversation
    if [[ -n "$input_text" ]]; then
        # Generate new conversation ID and switch to it
        local new_id=$($_FORGE_BIN conversation new)
        _forge_switch_conversation "$new_id"
        
        # Execute the forge command with the input text
        _forge_exec_interactive -p "$input_text" --cid "$_FORGE_CONVERSATION_ID"
        
        # Start background sync job if enabled and not already running
        _forge_start_background_sync
        # Start background update check
        _forge_start_background_update
    else
        # Only show banner if no input text (starting fresh conversation)
        _forge_exec banner
    fi
}

# Action handler: Show session info
function _forge_action_info() {
    echo
    if [[ -n "$_FORGE_CONVERSATION_ID" ]]; then
        _forge_exec info --cid "$_FORGE_CONVERSATION_ID"
    else
        _forge_exec info
    fi
}

# Action handler: Dump conversation
function _forge_action_dump() {
    local input_text="$1"
    if [[ "$input_text" == "html" ]]; then
        _forge_handle_conversation_command "dump" "--html"
    else
        _forge_handle_conversation_command "dump"
    fi
}

# Action handler: Compact conversation
function _forge_action_compact() {
    _forge_handle_conversation_command "compact"
}

# Action handler: Retry last message
function _forge_action_retry() {
    _forge_handle_conversation_command "retry"
}

# Helper function to handle conversation commands that require an active conversation
function _forge_handle_conversation_command() {
    local subcommand="$1"
    shift  # Remove first argument, remaining args become extra parameters
    
    echo
    
    # Check if FORGE_CONVERSATION_ID is set
    if [[ -z "$_FORGE_CONVERSATION_ID" ]]; then
        _forge_log error "No active conversation. Start a conversation first or use :conversation to see existing ones"
        return 0
    fi
    
    # Execute the conversation command with conversation ID and any extra arguments
    _forge_exec conversation "$subcommand" "$_FORGE_CONVERSATION_ID" "$@"
}
