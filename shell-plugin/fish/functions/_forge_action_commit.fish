# Forge: _forge_action_commit - Generate and apply commit
function _forge_action_commit
    set -l additional_context $argv[1]
    echo

    if test -n "$additional_context"
        env FORCE_COLOR=true CLICOLOR_FORCE=1 $_FORGE_BIN commit --max-diff $_FORGE_MAX_COMMIT_DIFF $additional_context
    else
        env FORCE_COLOR=true CLICOLOR_FORCE=1 $_FORGE_BIN commit --max-diff $_FORGE_MAX_COMMIT_DIFF
    end
end
