# Forge: _forge_action_info - Show current info
function _forge_action_info
    echo
    if test -n "$_FORGE_CONVERSATION_ID"
        _forge_exec info --cid $_FORGE_CONVERSATION_ID
    else
        _forge_exec info
    end
end
