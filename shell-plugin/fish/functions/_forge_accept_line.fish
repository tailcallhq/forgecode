# Forge: _forge_accept_line - Main enter key handler
# Intercepts :commands and `: <prompt>` patterns, passes everything else through
function _forge_accept_line
    set -l buf (commandline)

    # Check if the line matches `:command [args]` or `: text`
    # Pattern 1: :command_name [optional args]
    # Pattern 2: : freetext (prompt to forge)
    # Otherwise: pass through to normal execution

    set -l user_action ""
    set -l input_text ""

    if string match -rq '^:([a-zA-Z][a-zA-Z0-9_-]*)(\s+(.*))?$' -- "$buf"
        set user_action (string match -r '^:([a-zA-Z][a-zA-Z0-9_-]*)' -- "$buf" | tail -1)
        # Extract text after the command
        set input_text (string replace -r '^:[a-zA-Z][a-zA-Z0-9_-]*\s*' '' -- "$buf")
    else if string match -rq '^: (.+)$' -- "$buf"
        set user_action ""
        set input_text (string replace -r '^: ' '' -- "$buf")
    else
        # Normal command - execute normally
        commandline -f execute
        return
    end

    # Add to history (Fish 4.x uses "append", older versions use "merge")
    builtin history append -- "$buf" 2>/dev/null
    or builtin history merge 2>/dev/null

    # Handle aliases
    switch "$user_action"
        case ask
            set user_action sage
        case plan
            set user_action muse
    end

    # Clear the command line before running actions
    commandline -r ""
    commandline -f repaint

    # Dispatch to action handlers
    switch "$user_action"
        case new n
            _forge_action_new "$input_text"
        case info i
            _forge_action_info
        case env e
            _forge_action_env
        case dump d
            _forge_action_dump "$input_text"
        case compact
            _forge_action_compact
        case retry r
            _forge_action_retry
        case agent a
            _forge_action_agent "$input_text"
        case conversation c
            _forge_action_conversation "$input_text"
        case config-provider provider p
            _forge_action_provider "$input_text"
        case config-model cm
            _forge_action_model "$input_text"
        case model m
            _forge_action_session_model "$input_text"
        case config-reload cr model-reset mr
            _forge_action_config_reload
        case reasoning-effort re
            _forge_action_reasoning_effort "$input_text"
        case config-reasoning-effort cre
            _forge_action_config_reasoning_effort "$input_text"
        case config-commit-model ccm
            _forge_action_commit_model "$input_text"
        case config-suggest-model csm
            _forge_action_suggest_model "$input_text"
        case tools t
            _forge_action_tools
        case config
            _forge_action_config
        case config-edit ce
            _forge_action_config_edit
        case skill
            _forge_action_skill
        case edit ed
            _forge_action_editor "$input_text"
            return
        case commit
            _forge_action_commit "$input_text"
        case commit-preview
            _forge_action_commit_preview "$input_text"
            return
        case suggest s
            _forge_action_suggest "$input_text"
            return
        case clone
            _forge_action_clone "$input_text"
        case rename rn
            _forge_action_rename "$input_text"
        case conversation-rename
            _forge_action_conversation_rename "$input_text"
        case copy
            _forge_action_copy
        case workspace-sync sync
            _forge_action_sync
        case workspace-init sync-init
            _forge_action_sync_init
        case workspace-status sync-status
            _forge_action_sync_status
        case workspace-info sync-info
            _forge_action_sync_info
        case provider-login login
            _forge_action_login "$input_text"
        case logout
            _forge_action_logout "$input_text"
        case doctor
            _forge_action_doctor
        case keyboard-shortcuts kb
            _forge_action_keyboard
        case '*'
            _forge_action_default "$user_action" "$input_text"
    end

    commandline -r ""
    commandline -f repaint
end
