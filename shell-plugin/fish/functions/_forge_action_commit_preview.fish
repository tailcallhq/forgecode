# Forge: _forge_action_commit_preview - Preview commit message and place in command line
function _forge_action_commit_preview
    set -l additional_context $argv[1]
    echo

    set -l commit_message
    if test -n "$additional_context"
        set commit_message (env FORCE_COLOR=true CLICOLOR_FORCE=1 $_FORGE_BIN commit --preview --max-diff $_FORGE_MAX_COMMIT_DIFF $additional_context)
    else
        set commit_message (env FORCE_COLOR=true CLICOLOR_FORCE=1 $_FORGE_BIN commit --preview --max-diff $_FORGE_MAX_COMMIT_DIFF)
    end

    if test -n "$commit_message"
        if git diff --staged --quiet
            commandline -r "git commit -am '$commit_message'"
        else
            commandline -r "git commit -m '$commit_message'"
        end
        commandline -f repaint
    end
end
