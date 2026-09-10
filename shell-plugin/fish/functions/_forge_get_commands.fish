# Forge: _forge_get_commands - Get cached list of forge commands
function _forge_get_commands
    if test -z "$_FORGE_COMMANDS"
        set -g _FORGE_COMMANDS (env CLICOLOR_FORCE=0 $_FORGE_BIN list commands --porcelain 2>/dev/null | sed 's/Display ZSH keyboard shortcuts/Display Fish keyboard shortcuts/')
    end
    printf '%s\n' $_FORGE_COMMANDS
end
