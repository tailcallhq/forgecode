# Forge: _forge_action_doctor - Run diagnostics
function _forge_action_doctor
    echo
    # Note: We call the zsh doctor for now as forge doesn't have a fish-specific doctor.
    # It will still show useful general diagnostics.
    $_FORGE_BIN zsh doctor 2>/dev/null; or echo "Forge Fish plugin loaded. Shell: fish "(fish --version 2>&1)
end
