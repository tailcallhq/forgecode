# Forge: _forge_action_new - Start a new conversation
function _forge_action_new
    set -l input_text $argv[1]
    _forge_clear_conversation
    set -g _FORGE_ACTIVE_AGENT forge
    echo

    if test -n "$input_text"
        set -l new_id ($_FORGE_BIN conversation new)
        _forge_switch_conversation $new_id
        _forge_exec_interactive -p "$input_text" --cid $_FORGE_CONVERSATION_ID
        _forge_start_background_sync
        _forge_start_background_update
    else
        _forge_exec banner
    end
end
