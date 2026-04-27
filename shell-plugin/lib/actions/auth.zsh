#!/usr/bin/env zsh

# Authentication action handlers

# Action handler: Login to provider
function _forge_action_login() {
    local input_text="$1"
    echo

    local provider
    provider=$(_forge_select_with_query "$input_text" provider)

    if [[ -n "$provider" ]]; then
        _forge_exec_interactive provider login "$provider"
    fi
}

# Action handler: Logout from provider
function _forge_action_logout() {
    local input_text="$1"
    echo

    local provider
    provider=$(_forge_select_with_query "$input_text" provider --configured)

    if [[ -n "$provider" ]]; then
        _forge_exec provider logout "$provider"
    fi
}
