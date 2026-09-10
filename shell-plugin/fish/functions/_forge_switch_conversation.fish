# Forge: _forge_switch_conversation - Switch to a conversation, saving previous
function _forge_switch_conversation
    set -l new_id $argv[1]
    if test -n "$_FORGE_CONVERSATION_ID" -a "$_FORGE_CONVERSATION_ID" != "$new_id"
        set -g _FORGE_PREVIOUS_CONVERSATION_ID $_FORGE_CONVERSATION_ID
    end
    set -g _FORGE_CONVERSATION_ID $new_id
end
