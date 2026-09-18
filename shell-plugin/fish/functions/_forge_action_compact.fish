# Forge: _forge_action_compact - Compact conversation
function _forge_action_compact
    echo
    if test -z "$_FORGE_CONVERSATION_ID"
        _forge_log error "No active conversation. Start a conversation first or use :conversation to see existing ones"
        return 0
    end
    _forge_exec conversation compact $_FORGE_CONVERSATION_ID
end
