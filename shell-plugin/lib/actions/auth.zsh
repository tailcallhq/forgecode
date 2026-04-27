#!/usr/bin/env zsh

# Authentication action handlers

# Action handler: Login to provider
function _forge_action_login() {
    local input_text="$1"
    echo

    local provider
    if [[ -n "$input_text" ]]; then
        provider=$(_forge_select provider --query "$input_text")
    else
        provider=$(_forge_select provider)
    fi

    if [[ -n "$provider" ]]; then
        _forge_exec_interactive provider login "$provider"
    fi
}

# Action handler: Logout from provider
function _forge_action_logout() {
    local input_text="$1"
    echo

    local provider
    if [[ -n "$input_text" ]]; then
        provider=$(_forge_select provider --configured --query "$input_text")
    else
        provider=$(_forge_select provider --configured)
    fi

    if [[ -n "$provider" ]]; then
        _forge_exec provider logout "$provider"
    fi
}
