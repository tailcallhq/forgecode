# Main command dispatcher for forge fish plugin

# Action handler: Set active agent or execute command
function _forge_action_default
    set -l user_action "$argv[1]"
    set -l input_text "$argv[2]"
    set -l command_type ""

    # Validate that the command exists in show-commands (if user_action is provided)
    if test -n "$user_action"
        set -l commands_list (_forge_get_commands)
        if test -n "$commands_list"
            # Check if the user_action is in the list of valid commands and extract the row
            set -l command_row (echo "$commands_list" | grep "^$user_action"'\b')
            if test -z "$command_row"
                echo
                _forge_log error "Command '$user_action' not found"
                return 0
            end

            # Extract the command type from the second field (TYPE column)
            set command_type (echo "$command_row" | awk '{print $2}')
            # Case-insensitive comparison
            if test (string lower "$command_type") = "custom"
                # Generate conversation ID if needed
                if test -z "$_FORGE_CONVERSATION_ID"
                    set -l new_id ($_FORGE_BIN conversation new)
                    set -g _FORGE_CONVERSATION_ID "$new_id"
                end

                echo
                # Execute custom command with execute subcommand
                if test -n "$input_text"
                    _forge_exec cmd execute --cid "$_FORGE_CONVERSATION_ID" "$user_action" "$input_text"
                else
                    _forge_exec cmd execute --cid "$_FORGE_CONVERSATION_ID" "$user_action"
                end
                return 0
            end
        end
    end

    # If input_text is empty, just set the active agent (only for AGENT type commands)
    if test -z "$input_text"
        if test -n "$user_action"
            if test (string lower "$command_type") != "agent"
                echo
                _forge_log error "Command '$user_action' not found"
                return 0
            end
            echo
            # Set the agent in the global variable
            set -g _FORGE_ACTIVE_AGENT "$user_action"
            _forge_log info (string upper "$_FORGE_ACTIVE_AGENT")" is now the active agent"
        end
        return 0
    end

    # Generate conversation ID if needed
    if test -z "$_FORGE_CONVERSATION_ID"
        set -l new_id ($_FORGE_BIN conversation new)
        set -g _FORGE_CONVERSATION_ID "$new_id"
    end

    echo

    # Only set the agent if user explicitly specified one
    if test -n "$user_action"
        set -g _FORGE_ACTIVE_AGENT "$user_action"
    end

    # Execute the forge command directly with proper escaping
    set -lx FORGE_SHELL_PROMPT 1
    _forge_exec_interactive -p "$input_text" --cid "$_FORGE_CONVERSATION_ID"

    # Start background sync job if enabled and not already running
    set -l shell_sync_enabled (FORGE_SHELL_PROMPT=1 $_FORGE_BIN config get shell-behavior-sync 2>/dev/null)
    if test -n "$shell_sync_enabled"
        set -lx FORGE_SHELL_PROMPT 1
        set -lx FORGE_SHELL_BEHAVIOR_SYNC "$shell_sync_enabled"
        _forge_start_background_sync
    else
        set -lx FORGE_SHELL_PROMPT 1
        _forge_start_background_sync
    end
    # Start background update check
    _forge_start_background_update
end

# Main accept-line handler that intercepts :commands
function __forge_accept_line
    set -l cmd (commandline)

    # Check if the line starts with any of the supported patterns
    # Pattern 1: :command [args] (e.g., :new, :commit fix typo)
    set -l user_action ""
    set -l input_text ""

    if string match -qr '^:([a-zA-Z][a-zA-Z0-9_-]*)( (.*))?$' -- $cmd
        # Action with or without parameters: :foo or :foo bar baz
        set user_action (string replace -r '^:([a-zA-Z][a-zA-Z0-9_-]*).*$' '$1' -- $cmd)
        # Extract the rest after the command name
        set -l rest (string replace -r '^:[a-zA-Z][a-zA-Z0-9_-]*' '' -- $cmd)
        if string match -qr '^ (.+)$' -- $rest
            set input_text (string replace -r '^ ' '' -- $rest)
        else
            set input_text ""
        end
    else if string match -qr '^: (.*)$' -- $cmd
        # Default action with parameters: : something
        set user_action ""
        set input_text (string replace -r '^: ' '' -- $cmd)
    else
        # For non-:commands, use normal accept-line
        commandline -f execute
        return
    end

    # Add the original command to history before transformation
    # Fish adds to history via 'history merge' after execution, but we
    # can use 'history --merge' or just let it happen naturally
    # For now, we manually add it
    commandline -r ''
    commandline -f repaint

    # Handle aliases - convert to their actual agent names
    switch "$user_action"
        case ask
            set user_action "sage"
        case plan
            set user_action "muse"
    end

    # Dispatch to appropriate action handler
    switch "$user_action"
        case sage ask
            _forge_action_agent "sage"
        case muse plan
            _forge_action_agent "muse"
        case forge
            _forge_action_agent "forge"
        case new n
            _forge_action_new "$input_text"
        case info i
            _forge_action_info
        case dump d
            _forge_action_dump "$input_text"
        case compact
            _forge_action_compact
        case retry r
            _forge_action_retry
        case help
            _forge_action_help
        case agent a
            _forge_action_agent "$input_text"
        case conversation c
            _forge_action_conversation "$input_text"
        case config-model cm
            _forge_action_model "$input_text"
        case model m
            _forge_action_shell_model "$input_text"
        case config-reload cr model-reset mr
            _forge_action_config_reload
        case reasoning-effort re
            _forge_action_reasoning_effort "$input_text"
        case config-reasoning-effort cre
            _forge_action_config_reasoning_effort "$input_text"
        case config-shell-model cshm
            _forge_action_shell_model "$input_text"
        case config-commit-model ccm
            _forge_action_commit_model "$input_text"
        case config-suggest-model csm
            _forge_action_suggest_model "$input_text"
        case tools t
            _forge_action_tools
        case config env e
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
        case provider-login login provider
            _forge_action_login "$input_text"
        case logout
            _forge_action_logout "$input_text"
        case paste-image pi
            _forge_action_paste_image
        case '*'
            _forge_action_default "$user_action" "$input_text"
    end

    # Centralized reset after all actions complete
    _forge_reset
end
