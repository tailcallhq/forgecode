# Forge: _forge_action_editor - Open editor for multi-line prompt input
function _forge_action_editor
    set -l initial_text $argv[1]
    echo

    set -l editor_cmd (test -n "$FORGE_EDITOR"; and echo $FORGE_EDITOR; or test -n "$EDITOR"; and echo $EDITOR; or echo nano)
    set -l editor_bin (echo $editor_cmd | awk '{print $1}')

    if not command -q $editor_bin
        _forge_log error "Editor not found: $editor_cmd (set FORGE_EDITOR or EDITOR)"
        return 1
    end

    set -l forge_dir ".forge"
    if not test -d $forge_dir
        mkdir -p $forge_dir; or begin
            _forge_log error "Failed to create .forge directory"
            return 1
        end
    end

    set -l temp_file "$forge_dir/FORGE_EDITMSG.md"
    touch $temp_file; or begin
        _forge_log error "Failed to create temporary file"
        return 1
    end

    if test -n "$initial_text"
        printf '%s\n' $initial_text >$temp_file
    else
        echo -n "" >$temp_file
    end

    eval "$editor_cmd '$temp_file'" </dev/tty >/dev/tty 2>&1
    set -l editor_exit_code $status

    if test $editor_exit_code -ne 0
        _forge_log error "Editor exited with error code $editor_exit_code"
        return 1
    end

    set -l content (cat $temp_file | tr -d '\r')
    if test -z "$content"
        _forge_log info "Editor closed with no content"
        commandline -r ""
        commandline -f repaint
        return 0
    end

    commandline -r ": $content"
    commandline -f repaint
end
