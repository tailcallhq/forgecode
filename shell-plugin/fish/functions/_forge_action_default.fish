# Forge: _forge_action_default - Handle unknown/custom commands and direct prompts
function _forge_action_default
    set -l user_action $argv[1]
    set -l input_text $argv[2]
    set -l command_type ""

    if test -n "$user_action"
        set -l commands_list (_forge_get_commands)
        if test -n "$commands_list"
            set -l command_row (printf '%s\n' $commands_list | grep "^$user_action\\b")
            if test -z "$command_row"
                echo
                _forge_log error "Command '"(set_color --bold)"$user_action"(set_color normal)"' not found"
                return 0
            end
            set command_type (echo "$command_row" | awk '{print $2}')

            # Handle custom commands
            if test (string lower "$command_type") = custom
                if test -z "$_FORGE_CONVERSATION_ID"
                    set -g _FORGE_CONVERSATION_ID ($_FORGE_BIN conversation new)
                end
                echo
                if test -n "$input_text"
                    _forge_exec cmd execute --cid $_FORGE_CONVERSATION_ID $user_action "$input_text"
                else
                    _forge_exec cmd execute --cid $_FORGE_CONVERSATION_ID $user_action
                end
                return 0
            end
        end
    end

    if test -z "$input_text"
        if test -n "$user_action"
            if test (string lower "$command_type") != agent
                echo
                _forge_log error "Command '"(set_color --bold)"$user_action"(set_color normal)"' not found"
                return 0
            end
            echo
            set -g _FORGE_ACTIVE_AGENT $user_action
            _forge_log info (set_color --bold white)(string upper $user_action)(set_color normal)" "(set_color 888888)"is now the active agent"(set_color normal)
        end
        return 0
    end

    # Create conversation if needed
    if test -z "$_FORGE_CONVERSATION_ID"
        set -g _FORGE_CONVERSATION_ID ($_FORGE_BIN conversation new)
    end

    echo
    if test -n "$user_action"
        set -g _FORGE_ACTIVE_AGENT $user_action
    end
    _forge_exec_interactive -p "$input_text" --cid $_FORGE_CONVERSATION_ID
    _forge_start_background_sync
    _forge_start_background_update
end
