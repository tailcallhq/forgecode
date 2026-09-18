# Forge: _forge_action_logout - Interactive provider logout
function _forge_action_logout
    set -l input_text $argv[1]
    echo
    set -l selected (_forge_select_provider "\\[yes\\]" "" "" "$input_text")
    if test -n "$selected"
        set -l provider (echo "$selected" | awk '{print $2}')
        _forge_exec provider logout $provider
    end
end
