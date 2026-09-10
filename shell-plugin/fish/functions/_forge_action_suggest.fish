# Forge: _forge_action_suggest - Generate shell command from description
function _forge_action_suggest
    set -l description $argv[1]
    if test -z "$description"
        _forge_log error "Please provide a command description"
        return 0
    end
    echo

    set -lx FORCE_COLOR true
    set -lx CLICOLOR_FORCE 1
    set -l generated_command (_forge_exec suggest "$description")
    if test -n "$generated_command"
        commandline -r "$generated_command"
        commandline -f repaint
    else
        _forge_log error "Failed to generate command"
    end
end
