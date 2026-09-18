# Forge: _forge_action_tools - List tools for active agent
function _forge_action_tools
    echo
    set -l agent_id (test -n "$_FORGE_ACTIVE_AGENT"; and echo $_FORGE_ACTIVE_AGENT; or echo forge)
    _forge_exec list tools $agent_id
end
