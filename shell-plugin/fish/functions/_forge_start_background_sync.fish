# Forge: _forge_start_background_sync - Background workspace sync
function _forge_start_background_sync
    set -l sync_enabled (test -n "$FORGE_SYNC_ENABLED"; and echo $FORGE_SYNC_ENABLED; or echo true)
    if test "$sync_enabled" != true
        return 0
    end

    set -l workspace_path (pwd -P)

    # Run in background: check if workspace is indexed, then sync
    fish -c "
        if $_FORGE_BIN workspace info '$workspace_path' >/dev/null 2>&1
            $_FORGE_BIN workspace sync '$workspace_path' >/dev/null 2>&1
        end
    " &
    disown
end
