# Forge: _forge_exec - Execute forge command with session overrides
function _forge_exec
    set -l agent_id (test -n "$_FORGE_ACTIVE_AGENT"; and echo $_FORGE_ACTIVE_AGENT; or echo forge)
    set -l cmd $_FORGE_BIN --agent $agent_id $argv

    # Build env prefix for session overrides
    # Fish's `set -lx` inside if-blocks is block-scoped, so we use `env` command
    set -l env_args
    if test -n "$_FORGE_SESSION_MODEL"
        set -a env_args FORGE_SESSION__MODEL_ID=$_FORGE_SESSION_MODEL
    end
    if test -n "$_FORGE_SESSION_PROVIDER"
        set -a env_args FORGE_SESSION__PROVIDER_ID=$_FORGE_SESSION_PROVIDER
    end
    if test -n "$_FORGE_SESSION_REASONING_EFFORT"
        set -a env_args FORGE_REASONING__EFFORT=$_FORGE_SESSION_REASONING_EFFORT
    end

    if test (count $env_args) -gt 0
        env $env_args $cmd
    else
        $cmd
    end
end
