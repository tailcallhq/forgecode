# Forge: _forge_action_suggest_model - Set suggest model
function _forge_action_suggest_model
    set -l input_text $argv[1]
    echo

    set -l suggest_output (_forge_exec config get suggest 2>/dev/null)
    set -l current_suggest_provider (printf '%s\n' $suggest_output | head -n 1)
    set -l current_suggest_model (printf '%s\n' $suggest_output | tail -n 1)

    set -l selected (_forge_pick_model "Suggest Model ❯ " "$current_suggest_model" "$input_text" "$current_suggest_provider" 4)
    if test -n "$selected"
        set -l model_id (echo "$selected" | awk -F '  +' '{print $1}' | string trim)
        set -l provider_id (echo "$selected" | awk -F '  +' '{print $4}' | string trim)
        _forge_exec config set suggest $provider_id $model_id
    end
end
