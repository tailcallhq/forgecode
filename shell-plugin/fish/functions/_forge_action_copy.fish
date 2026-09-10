# Forge: _forge_action_copy - Copy last assistant message to clipboard
function _forge_action_copy
    echo
    if test -z "$_FORGE_CONVERSATION_ID"
        _forge_log error "No active conversation. Start a conversation first or use :conversation to see existing ones"
        return 0
    end

    set -l content ($_FORGE_BIN conversation show --md $_FORGE_CONVERSATION_ID 2>/dev/null)
    if test -z "$content"
        _forge_log error "No assistant message found in the current conversation"
        return 0
    end

    if command -q pbcopy
        printf '%s\n' $content | pbcopy
    else if command -q xclip
        printf '%s\n' $content | xclip -selection clipboard
    else if command -q xsel
        printf '%s\n' $content | xsel --clipboard --input
    else
        _forge_log error "No clipboard utility found (pbcopy, xclip, or xsel required)"
        return 0
    end

    set -l line_count (printf '%s\n' $content | wc -l | string trim)
    set -l byte_count (printf '%s\n' $content | wc -c | string trim)
    _forge_log success "Copied to clipboard "(set_color 888888)"[$line_count lines, $byte_count bytes]"(set_color normal)
end
