# Forge: _forge_action_provider - Switch provider interactively
function _forge_action_provider
    set -l input_text $argv[1]
    echo
    set -l selected (_forge_select_provider "" "" "llm" "$input_text")
    if test -n "$selected"
        set -l provider_id (echo "$selected" | awk '{print $2}')
        _forge_exec_interactive config set provider $provider_id
    end
end
