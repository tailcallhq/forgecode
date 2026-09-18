# Forge: _forge_action_clone - Clone a conversation
function _forge_action_clone
    set -l input_text $argv[1]
    echo

    if test -n "$input_text"
        _forge_clone_and_switch $input_text
        return 0
    end

    set -l conversations_output ($_FORGE_BIN conversation list --porcelain 2>/dev/null)
    if test -z "$conversations_output"
        _forge_log error "No conversations found"
        return 0
    end

    set -l current_id $_FORGE_CONVERSATION_ID
    set -l fzf_args \
        --prompt="Clone Conversation ❯ " \
        --delimiter="$_FORGE_DELIMITER" \
        --with-nth="2,3" \
        --preview="CLICOLOR_FORCE=1 $_FORGE_BIN conversation info {1}; echo; CLICOLOR_FORCE=1 $_FORGE_BIN conversation show {1}" \
        $_FORGE_PREVIEW_WINDOW

    if test -n "$current_id"
        set -l idx (printf '%s\n' $conversations_output | _forge_find_index "$current_id")
        set fzf_args $fzf_args --bind="start:pos($idx)"
    end

    set -l selected (printf '%s\n' $conversations_output | _forge_fzf --header-lines=1 $fzf_args)
    if test -n "$selected"
        set -l conversation_id (echo "$selected" | sed -E 's/  .*//' | tr -d '\n')
        _forge_clone_and_switch $conversation_id
    end
end
