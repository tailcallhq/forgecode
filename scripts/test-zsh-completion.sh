#!/usr/bin/env zsh

# Regression test for preserving the saved Tab widget's ZLE context.
#
# Usage: zsh scripts/test-zsh-completion.sh

if [[ "${FORGE_COMPLETION_TEST_INNER:-}" == "1" ]]; then
    FORGE_TEST_PLUGIN_LIB="${0:A:h}/../shell-plugin/lib"

    PS1=
    PROMPT=
    RPROMPT=

    function something-else() {
        _FORGE_TEST_WIDGET="$WIDGET"
    }
    zle -N something-else
    bindkey '^I' something-else

    source "${FORGE_TEST_PLUGIN_LIB}/completion.zsh"
    source "${FORGE_TEST_PLUGIN_LIB}/bindings.zsh"

    function forge-test-driver() {
        BUFFER="cd"
        LBUFFER="$BUFFER"
        RBUFFER=""
        CURSOR=${#BUFFER}
        zle forge-completion
        print -r -- "FORGE_WIDGET=${_FORGE_TEST_WIDGET:-}"
        print -r -- "SENTINEL_DONE"
        exit 0
    }
    zle -N forge-test-driver

    function zle-line-init() {
        [[ -n "${_FORGE_TEST_DRIVER_STARTED:-}" ]] && return 0
        _FORGE_TEST_DRIVER_STARTED=1
        zle forge-test-driver
    }
    zle -N zle-line-init

    vared -c __forge_test_line
    exit 1
fi

set -euo pipefail
zmodload zsh/zpty

SCRIPT_PATH="${0:A}"
PTY_NAME="forge_tab_context_$$"
PTY_OUTPUT=""

zpty -b "$PTY_NAME" env \
    FORGE_COMPLETION_TEST_INNER=1 \
    zsh -f -i "$SCRIPT_PATH"

deadline=$(( SECONDS + 5 ))
while (( SECONDS < deadline )); do
    if zpty -r "$PTY_NAME" chunk; then
        PTY_OUTPUT+="$chunk"
        [[ "$PTY_OUTPUT" == *SENTINEL_DONE* ]] && break
    else
        zpty -t "$PTY_NAME" || break
        sleep 0.05
    fi
done
zpty -d "$PTY_NAME" 2>/dev/null || true

if [[ "$PTY_OUTPUT" == *"FORGE_WIDGET=something-else"* ]]; then
    print -r -- "PASS: saved Tab widget receives its own WIDGET context"
else
    print -r -- "FAIL: saved Tab widget did not receive its own WIDGET context"
    print -r -- "$PTY_OUTPUT"
    exit 1
fi
