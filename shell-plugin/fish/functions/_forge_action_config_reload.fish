# Forge: _forge_action_config_reload - Clear session overrides
function _forge_action_config_reload
    echo
    if test -z "$_FORGE_SESSION_MODEL" -a -z "$_FORGE_SESSION_PROVIDER" -a -z "$_FORGE_SESSION_REASONING_EFFORT"
        _forge_log info "No session overrides active (already using global config)"
        return 0
    end
    set -g _FORGE_SESSION_MODEL ""
    set -g _FORGE_SESSION_PROVIDER ""
    set -g _FORGE_SESSION_REASONING_EFFORT ""
    _forge_log success "Session overrides cleared — using global config"
end
