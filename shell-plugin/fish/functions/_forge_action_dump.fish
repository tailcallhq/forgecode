# Forge: _forge_action_dump - Dump conversation
function _forge_action_dump
    set -l input_text $argv[1]
    echo
    if test -z "$_FORGE_CONVERSATION_ID"
        _forge_log error "No active conversation. Start a conversation first or use :conversation to see existing ones"
        return 0
    end
    if test "$input_text" = html
        _forge_exec conversation dump $_FORGE_CONVERSATION_ID --html
    else
        _forge_exec conversation dump $_FORGE_CONVERSATION_ID
    end
end
