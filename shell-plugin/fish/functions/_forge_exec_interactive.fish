# Forge: _forge_exec_interactive - Execute forge command interactively (with TTY)
function _forge_exec_interactive
    set -l agent_id (test -n "$_FORGE_ACTIVE_AGENT"; and echo $_FORGE_ACTIVE_AGENT; or echo forge)
    set -l cmd $_FORGE_BIN --agent $agent_id $argv

    # Build env prefix for session overrides
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
        env $env_args $cmd </dev/tty >/dev/tty
    else
        $cmd </dev/tty >/dev/tty
    end
end
