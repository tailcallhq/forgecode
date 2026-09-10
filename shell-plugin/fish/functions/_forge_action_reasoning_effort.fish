# Forge: _forge_action_reasoning_effort - Set session reasoning effort
function _forge_action_reasoning_effort
    set -l input_text $argv[1]
    echo

    set -l efforts "EFFORT\nnone\nminimal\nlow\nmedium\nhigh\nxhigh\nmax"
    set -l current_effort
    if test -n "$_FORGE_SESSION_REASONING_EFFORT"
        set current_effort $_FORGE_SESSION_REASONING_EFFORT
    else
        set current_effort ($_FORGE_BIN config get reasoning-effort 2>/dev/null)
    end

    set -l fzf_args --prompt="Reasoning Effort ❯ "
    if test -n "$input_text"
        set fzf_args $fzf_args --query="$input_text"
    end
    if test -n "$current_effort"
        set -l idx (echo -e $efforts | _forge_find_index "$current_effort" 1)
        set fzf_args $fzf_args --bind="start:pos($idx)"
    end

    set -l selected (echo -e $efforts | _forge_fzf --header-lines=1 $fzf_args)
    if test -n "$selected"
        set -g _FORGE_SESSION_REASONING_EFFORT $selected
        _forge_log success "Session reasoning effort set to "(set_color --bold)"$selected"(set_color normal)
    end
end
