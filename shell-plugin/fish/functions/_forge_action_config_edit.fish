# Forge: _forge_action_config_edit - Edit forge config in $EDITOR
function _forge_action_config_edit
    echo
    set -l editor_cmd (test -n "$FORGE_EDITOR"; and echo $FORGE_EDITOR; or test -n "$EDITOR"; and echo $EDITOR; or echo nano)
    set -l editor_bin (echo $editor_cmd | awk '{print $1}')

    if not command -q $editor_bin
        _forge_log error "Editor not found: $editor_cmd (set FORGE_EDITOR or EDITOR)"
        return 1
    end

    set -l config_file "$HOME/forge/.forge.toml"
    if not test -d "$HOME/forge"
        mkdir -p "$HOME/forge"; or begin
            _forge_log error "Failed to create ~/forge directory"
            return 1
        end
    end
    if not test -f "$config_file"
        touch "$config_file"; or begin
            _forge_log error "Failed to create $config_file"
            return 1
        end
    end

    eval "$editor_cmd '$config_file'" </dev/tty >/dev/tty 2>&1
    set -l exit_code $status
    if test $exit_code -ne 0
        _forge_log error "Editor exited with error code $exit_code"
    end
end
