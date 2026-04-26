#!/usr/bin/env zsh

# Authentication action handlers

# Action handler: Login to provider
function _forge_action_login() {
    local input_text="$1"
    echo

    local provider
    provider=$(CLICOLOR_FORCE=0 $_FORGE_BIN select provider ${input_text:+--query "$input_text"} </dev/tty 2>/dev/tty)

    if [[ -n "$provider" ]]; then
        _forge_exec_interactive provider login "$provider"
    fi
}

# Action handler: Logout from provider
function _forge_action_logout() {
    local input_text="$1"
    echo

    local provider
    provider=$(CLICOLOR_FORCE=0 $_FORGE_BIN select provider --configured ${input_text:+--query "$input_text"} </dev/tty 2>/dev/tty)

    if [[ -n "$provider" ]]; then
        _forge_exec provider logout "$provider"
    fi
}
