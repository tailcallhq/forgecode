# Forge: _forge_action_model - Set config model interactively
function _forge_action_model
    set -l input_text $argv[1]
    echo

    set -l current_model ($_FORGE_BIN config get model 2>/dev/null)
    set -l current_provider ($_FORGE_BIN config get provider 2>/dev/null)

    set -l selected (_forge_pick_model "Model ❯ " "$current_model" "$input_text" "$current_provider" 3)
    if test -n "$selected"
        set -l model_id (echo "$selected" | awk -F '  +' '{print $1}' | string trim)
        set -l provider_display (echo "$selected" | awk -F '  +' '{print $3}' | string trim)
        set -l provider_id (echo "$selected" | awk -F '  +' '{print $4}' | string trim)

        if test -n "$provider_display" -a "$provider_display" != "$current_provider"
            _forge_exec_interactive config set provider $provider_id --model $model_id
            return
        end
        _forge_exec config set model $model_id
    end
end
