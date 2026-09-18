# Forge: _forge_action_retry - Retry last conversation turn
function _forge_action_retry
    echo
    if test -z "$_FORGE_CONVERSATION_ID"
        _forge_log error "No active conversation. Start a conversation first or use :conversation to see existing ones"
        return 0
    end
    _forge_exec conversation retry $_FORGE_CONVERSATION_ID
end
