# Forge: _forge_start_background_update - Background auto-update check
function _forge_start_background_update
    fish -c "$_FORGE_BIN update --no-confirm >/dev/null 2>&1" &
    disown
end
