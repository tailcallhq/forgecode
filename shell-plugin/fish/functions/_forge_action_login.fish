# Forge: _forge_action_login - Interactive provider login
function _forge_action_login
    set -l input_text $argv[1]
    echo
    set -l selected (_forge_select_provider "" "" "" "$input_text")
    if test -n "$selected"
        set -l provider (echo "$selected" | awk '{print $2}')
        _forge_exec_interactive provider login $provider
    end
end
