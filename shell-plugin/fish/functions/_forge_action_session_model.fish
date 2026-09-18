# Forge: _forge_action_session_model - Set session model (non-persistent)
function _forge_action_session_model
    set -l input_text $argv[1]
    echo

    set -l current_model
    set -l current_provider
    set -l provider_index

    if test -n "$_FORGE_SESSION_MODEL"
        set current_model $_FORGE_SESSION_MODEL
        set provider_index 4
    else
        set current_model ($_FORGE_BIN config get model 2>/dev/null)
        set provider_index 3
    end

    if test -n "$_FORGE_SESSION_PROVIDER"
        set current_provider $_FORGE_SESSION_PROVIDER
        set provider_index 4
    else
        set current_provider ($_FORGE_BIN config get provider 2>/dev/null)
        set provider_index 3
    end

    set -l selected (_forge_pick_model "Session Model ❯ " "$current_model" "$input_text" "$current_provider" "$provider_index")
    if test -n "$selected"
        set -l model_id (echo "$selected" | awk -F '  +' '{print $1}' | string trim)
        set -l provider_id (echo "$selected" | awk -F '  +' '{print $4}' | string trim)
        set -g _FORGE_SESSION_MODEL $model_id
        set -g _FORGE_SESSION_PROVIDER $provider_id
        _forge_log success "Session model set to "(set_color --bold)"$model_id"(set_color normal)" (provider: "(set_color --bold)"$provider_id"(set_color normal)")"
    end
end
