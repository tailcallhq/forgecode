# Forge: _forge_action_conversation_rename - Rename any conversation
function _forge_action_conversation_rename
    set -l input_text $argv[1]
    echo

    if test -n "$input_text"
        set -l conversation_id (echo "$input_text" | awk '{print $1}')
        set -l new_name (echo "$input_text" | awk '{$1=""; print $0}' | string trim)
        if test "$conversation_id" = "$new_name" -o -z "$new_name"
            _forge_log error "Usage: :conversation-rename <id> <name>"
            return 0
        end
        _forge_exec conversation rename $conversation_id $new_name
        return 0
    end

    set -l conversations_output ($_FORGE_BIN conversation list --porcelain 2>/dev/null)
    if test -z "$conversations_output"
        _forge_log error "No conversations found"
        return 0
    end

    set -l current_id $_FORGE_CONVERSATION_ID
    set -l fzf_args \
        --prompt="Rename Conversation ❯ " \
        --delimiter="$_FORGE_DELIMITER" \
        --with-nth="2,3" \
        --preview="CLICOLOR_FORCE=1 $_FORGE_BIN conversation info {1}; echo; CLICOLOR_FORCE=1 $_FORGE_BIN conversation show {1}" \
        $_FORGE_PREVIEW_WINDOW

    if test -n "$current_id"
        set -l idx (printf '%s\n' $conversations_output | _forge_find_index "$current_id" 1)
        set fzf_args $fzf_args --bind="start:pos($idx)"
    end

    set -l selected (printf '%s\n' $conversations_output | _forge_fzf --header-lines=1 $fzf_args)
    if test -n "$selected"
        set -l conversation_id (echo "$selected" | sed -E 's/  .*//' | tr -d '\n')
        echo -n "Enter new name: "
        read new_name
        if test -n "$new_name"
            _forge_exec conversation rename $conversation_id $new_name
        else
            _forge_log error "No name provided, rename cancelled"
        end
    end
end
