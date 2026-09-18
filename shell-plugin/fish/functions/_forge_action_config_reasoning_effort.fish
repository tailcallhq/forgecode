# Forge: _forge_action_config_reasoning_effort - Set config reasoning effort (persistent)
function _forge_action_config_reasoning_effort
    set -l input_text $argv[1]
    echo

    set -l efforts "EFFORT\nnone\nminimal\nlow\nmedium\nhigh\nxhigh\nmax"
    set -l current_effort ($_FORGE_BIN config get reasoning-effort 2>/dev/null)

    set -l fzf_args --prompt="Config Reasoning Effort ❯ "
    if test -n "$input_text"
        set fzf_args $fzf_args --query="$input_text"
    end
    if test -n "$current_effort"
        set -l idx (echo -e $efforts | _forge_find_index "$current_effort" 1)
        set fzf_args $fzf_args --bind="start:pos($idx)"
    end

    set -l selected (echo -e $efforts | _forge_fzf --header-lines=1 $fzf_args)
    if test -n "$selected"
        _forge_exec config set reasoning-effort $selected
    end
end
