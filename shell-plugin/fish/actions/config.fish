# Configuration action handlers (agent, provider, model, tools, skill)

# Action handler: Select agent
function _forge_action_agent
    set -l input_text "$argv[1]"

    echo

    # If an agent ID is provided directly, use it
    if test -n "$input_text"
        set -l agent_id "$input_text"

        # Validate that the agent exists (skip header line)
        set -l agent_exists ($_FORGE_BIN list agents --porcelain 2>/dev/null | tail -n +2 | grep -q "^$agent_id"'\b'; and echo true; or echo false)
        if test "$agent_exists" = "false"
            _forge_log error "Agent '$agent_id' not found"
            return 0
        end

        # Set the agent as active
        set -g _FORGE_ACTIVE_AGENT "$agent_id"

        # Print log about agent switching
        _forge_log success "Switched to agent $agent_id"

        return 0
    end

    # Get agents list
    set -l agents_output
    set agents_output ($_FORGE_BIN list agents --porcelain 2>/dev/null)

    if test -n "$agents_output"
        # Get current agent ID
        set -l current_agent "$_FORGE_ACTIVE_AGENT"

        set -l sorted_agents "$agents_output"

        # Create prompt with current agent
        set -l prompt_text "Agent ❯ "
        set -l fzf_args --prompt="$prompt_text" --delimiter="$_FORGE_DELIMITER" --with-nth="1,2,4,5,6"

        # If there's a current agent, position cursor on it
        if test -n "$current_agent"
            set -l index (_forge_find_index "$sorted_agents" "$current_agent")
            set fzf_args $fzf_args --bind="start:pos($index)"
        end

        set -l selected_agent
        # Use fzf without preview for simple selection
        selected_agent (echo "$sorted_agents" | _forge_fzf --header-lines=1 $fzf_args)

        if test -n "$selected_agent"
            # Extract the first field (agent ID)
            set -l agent_id (echo "$selected_agent" | awk '{print $1}')

            # Set the selected agent as active
            set -g _FORGE_ACTIVE_AGENT "$agent_id"

            # Print log about agent switching
            _forge_log success "Switched to agent $agent_id"
        end
    else
        _forge_log error "No agents found"
    end
end

# Helper: Open an fzf model picker and print the raw selected line.
function _forge_pick_model
    set -l prompt_text "$argv[1]"
    set -l current_model "$argv[2]"
    set -l input_text "$argv[3]"
    set -l current_provider "$argv[4]"
    set -l provider_field "$argv[5]"

    set -l raw_output (env CLICOLOR_FORCE=0 NO_COLOR=1 TERM=dumb $_FORGE_BIN list models --porcelain </dev/null 2>/dev/null | string collect)
    set -l output (printf '%s\n' "$raw_output" | string replace -a \r \n | awk 'BEGIN { seen = 0 } /^ID[[:space:]]+MODEL[[:space:]]+PROVIDER/ { seen = 1 } seen { print }' | string collect)

    if test -z "$output"; and test -n "$raw_output"
        set output "$raw_output"
    end

    if test -z "$output"
        return 1
    end

    set -l fzf_args --delimiter="$_FORGE_DELIMITER" --prompt="$prompt_text" --with-nth="2,3,5.."

    if test -n "$input_text"
        set fzf_args $fzf_args --query="$input_text"
    end

    if test -n "$current_model"
        set -l index
        if test -n "$current_provider" -a -n "$provider_field"
            set index (_forge_find_index "$output" "$current_model" 1 "$provider_field" "$current_provider")
        else
            set index (_forge_find_index "$output" "$current_model" 1)
        end
        set fzf_args $fzf_args --bind="start:pos($index)"
    end

    printf '%s\n' "$output" | _forge_fzf --header-lines=1 $fzf_args
end

# Action handler: Select model (across all configured providers)
function _forge_action_model
    set -l input_text "$argv[1]"
    begin
        echo
        set -l current_model current_provider
        set current_model ($_FORGE_BIN config get model 2>/dev/null)
        set current_provider ($_FORGE_BIN config get provider 2>/dev/null)
        set -l selected
        set selected (_forge_pick_model "Model ❯ " "$current_model" "$input_text" "$current_provider" 3)

        if test -n "$selected"
            set -l model_id (echo "$selected" | awk -F '  +' '{print $1}')
            set -l provider_display (echo "$selected" | awk -F '  +' '{print $3}')
            set -l provider_id (echo "$selected" | awk -F '  +' '{print $4}')
            set model_id (string replace -a ' ' '' -- $model_id)
            set provider_id (string replace -a ' ' '' -- $provider_id)
            set provider_display (string replace -a ' ' '' -- $provider_display)

            # Switch provider first if it differs from the current one
            if test -n "$provider_display" -a "$provider_display" != "$current_provider"
                _forge_exec_interactive config set model "$provider_id" "$model_id"
                return
            end
            _forge_exec config set model "$provider_id" "$model_id"
        end
    end
end

# Action handler: Select model for shell mode
# Persists to config via `forge config set shell` and sets session variables
# so the current terminal session uses the new model immediately.
function _forge_action_shell_model
    set -l input_text "$argv[1]"
    echo

    set -l selected
    set selected (_forge_pick_model "Shell Model ❯ " "" "$input_text")

    if test -n "$selected"
        set -l model_id (echo "$selected" | awk -F '  +' '{print $1}')
        set -l provider_id (echo "$selected" | awk -F '  +' '{print $4}')
        set model_id (string replace -a ' ' '' -- $model_id)
        set provider_id (string replace -a ' ' '' -- $provider_id)

        set -g _FORGE_SESSION_MODEL "$model_id"
        set -g _FORGE_SESSION_PROVIDER "$provider_id"

        _forge_exec config set shell "$provider_id" "$model_id"
    end
end

# Action handler: Select model for commit message generation
function _forge_action_commit_model
    set -l input_text "$argv[1]"
    begin
        echo
        set -l commit_output current_commit_model current_commit_provider
        set commit_output (_forge_exec config get commit 2>/dev/null)
        set current_commit_provider (echo "$commit_output" | head -n 1)
        set current_commit_model (echo "$commit_output" | tail -n 1)

        set -l selected
        set selected (_forge_pick_model "Commit Model ❯ " "$current_commit_model" "$input_text" "$current_commit_provider" 4)

        if test -n "$selected"
            set -l model_id (echo "$selected" | awk -F '  +' '{print $1}')
            set -l provider_id (echo "$selected" | awk -F '  +' '{print $4}')
            set model_id (string replace -a ' ' '' -- $model_id)
            set provider_id (string replace -a ' ' '' -- $provider_id)

            _forge_exec config set commit "$provider_id" "$model_id"
        end
    end
end

# Action handler: Select model for command suggestion generation
function _forge_action_suggest_model
    set -l input_text "$argv[1]"
    begin
        echo
        set -l suggest_output current_suggest_model current_suggest_provider
        set suggest_output (_forge_exec config get suggest 2>/dev/null)
        set current_suggest_provider (echo "$suggest_output" | head -n 1)
        set current_suggest_model (echo "$suggest_output" | tail -n 1)

        set -l selected
        set selected (_forge_pick_model "Suggest Model ❯ " "$current_suggest_model" "$input_text" "$current_suggest_provider" 4)

        if test -n "$selected"
            set -l model_id (echo "$selected" | awk -F '  +' '{print $1}')
            set -l provider_id (echo "$selected" | awk -F '  +' '{print $4}')
            set model_id (string replace -a ' ' '' -- $model_id)
            set provider_id (string replace -a ' ' '' -- $provider_id)

            _forge_exec config set suggest "$provider_id" "$model_id"
        end
    end
end

# Action handler: Sync workspace for codebase search
function _forge_action_sync
    echo
    _forge_exec_interactive workspace sync --init
end

# Action handler: inits workspace for codebase search
function _forge_action_sync_init
    echo
    _forge_exec_interactive workspace init
end

# Action handler: Show sync status of workspace files
function _forge_action_sync_status
    echo
    _forge_exec workspace status "."
end

# Action handler: Show workspace info with sync details
function _forge_action_sync_info
    echo
    _forge_exec workspace info "."
end

# Helper function to select and set config values with fzf
function _forge_select_and_set_config
    set -l show_command "$argv[1]"
    set -l config_flag "$argv[2]"
    set -l prompt_text "$argv[3]"
    set -l default_value "$argv[4]"
    set -l with_nth "$argv[5]"
    set -l query "$argv[6]"

    begin
        echo
        set -l output
        # Handle multi-word commands properly
        set -l cmd_parts (string split ' ' -- $show_command)
        set output ($_FORGE_BIN $cmd_parts --porcelain 2>/dev/null)

        if test -n "$output"
            set -l fzf_args --delimiter="$_FORGE_DELIMITER" --prompt="$prompt_text ❯ "

            if test -n "$with_nth"
                set fzf_args $fzf_args --with-nth="$with_nth"
            end

            if test -n "$query"
                set fzf_args $fzf_args --query="$query"
            end

            if test -n "$default_value"
                set -l index (_forge_find_index "$output" "$default_value" 1)
                set fzf_args $fzf_args --bind="start:pos($index)"
            end

            set -l selected
            set selected (echo "$output" | _forge_fzf --header-lines=1 $fzf_args)

            if test -n "$selected"
                set -l name (echo "$selected" | string replace -r ' .*' '')
                _forge_exec config set "$config_flag" "$name"
            end
        end
    end
end

# Action handler: Select model for the current session only
function _forge_action_session_model
    set -l input_text "$argv[1]"
    echo

    set -l current_model current_provider provider_index
    if test -n "$_FORGE_SESSION_MODEL"
        set current_model "$_FORGE_SESSION_MODEL"
        set provider_index 4
    else
        set current_model ($_FORGE_BIN config get model 2>/dev/null)
        set provider_index 3
    end
    if test -n "$_FORGE_SESSION_PROVIDER"
        set current_provider "$_FORGE_SESSION_PROVIDER"
        set provider_index 4
    else
        set current_provider ($_FORGE_BIN config get provider 2>/dev/null)
        set provider_index 3
    end

    set -l selected
    set selected (_forge_pick_model "Session Model ❯ " "$current_model" "$input_text" "$current_provider" "$provider_index")

    if test -n "$selected"
        set -l model_id (echo "$selected" | awk -F '  +' '{print $1}')
        set -l provider_display (echo "$selected" | awk -F '  +' '{print $3}')
        set -l provider_id (echo "$selected" | awk -F '  +' '{print $4}')
        set model_id (string replace -a ' ' '' -- $model_id)
        set provider_id (string replace -a ' ' '' -- $provider_id)

        set -g _FORGE_SESSION_MODEL "$model_id"
        set -g _FORGE_SESSION_PROVIDER "$provider_id"

        _forge_exec config set model "$provider_id" "$model_id"

        _forge_log success "Session model set to $model_id (provider: $provider_id)"
    end
end

# Action handler: Reload config by resetting all session-scoped overrides
function _forge_action_config_reload
    echo

    if test -z "$_FORGE_SESSION_MODEL" -a -z "$_FORGE_SESSION_PROVIDER" -a -z "$_FORGE_SESSION_REASONING_EFFORT"
        _forge_log info "No session overrides active (already using global config)"
        return 0
    end

    set -g _FORGE_SESSION_MODEL ""
    set -g _FORGE_SESSION_PROVIDER ""
    set -g _FORGE_SESSION_REASONING_EFFORT ""

    _forge_log success "Session overrides cleared — using global config"
end

# Action handler: Select reasoning effort for the current session only
function _forge_action_reasoning_effort
    set -l input_text "$argv[1]"
    echo

    set -l efforts "EFFORT
none
minimal
low
medium
high
xhigh
max"

    set -l current_effort
    if test -n "$_FORGE_SESSION_REASONING_EFFORT"
        set current_effort "$_FORGE_SESSION_REASONING_EFFORT"
    else
        set current_effort ($_FORGE_BIN config get reasoning-effort 2>/dev/null)
    end

    set -l fzf_args --prompt="Reasoning Effort ❯ "

    if test -n "$input_text"
        set fzf_args $fzf_args --query="$input_text"
    end

    if test -n "$current_effort"
        set -l index (_forge_find_index "$efforts" "$current_effort" 1)
        set fzf_args $fzf_args --bind="start:pos($index)"
    end

    set -l selected
    set selected (echo "$efforts" | _forge_fzf --header-lines=1 $fzf_args)

    if test -n "$selected"
        set -g _FORGE_SESSION_REASONING_EFFORT "$selected"
        _forge_log success "Session reasoning effort set to $selected"
    end
end

# Action handler: Set reasoning effort in global config
function _forge_action_config_reasoning_effort
    set -l input_text "$argv[1]"
    begin
        echo

        set -l efforts "EFFORT
none
minimal
low
medium
high
xhigh
max"

        set -l current_effort
        set current_effort ($_FORGE_BIN config get reasoning-effort 2>/dev/null)

        set -l fzf_args --prompt="Config Reasoning Effort ❯ "

        if test -n "$input_text"
            set fzf_args $fzf_args --query="$input_text"
        end

        if test -n "$current_effort"
            set -l index (_forge_find_index "$efforts" "$current_effort" 1)
            set fzf_args $fzf_args --bind="start:pos($index)"
        end

        set -l selected
        set selected (echo "$efforts" | _forge_fzf --header-lines=1 $fzf_args)

        if test -n "$selected"
            _forge_exec config set reasoning-effort "$selected"
        end
    end
end

# Action handler: Show config list
function _forge_action_config
    echo
    $_FORGE_BIN config list
end

# Action handler: Open the global forge config file in an editor
function _forge_action_config_edit
    echo

    # Determine editor in order of preference: FORGE_EDITOR > EDITOR > nano
    set -l editor_cmd "$FORGE_EDITOR"
    if test -z "$editor_cmd"
        set editor_cmd "$EDITOR"
        if test -z "$editor_cmd"
            set editor_cmd nano
        end
    end

    # Validate editor exists
    if not command -v (string split ' ' -- $editor_cmd)[1] &>/dev/null
        _forge_log error "Editor not found: $editor_cmd (set FORGE_EDITOR or EDITOR)"
        return 1
    end

    # Resolve config file path via the forge binary
    set -l config_file
    set config_file ($FORGE_BIN config path 2>/dev/null)
    if test -z "$config_file"
        _forge_log error "Failed to resolve config path from '$FORGE_BIN config path'"
        return 1
    end

    set -l config_dir (dirname "$config_file")

    # Ensure the config directory exists
    if not test -d "$config_dir"
        mkdir -p "$config_dir"
        or begin
            _forge_log error "Failed to create $config_dir directory"
            return 1
        end
    end

    # Create the config file if it does not yet exist
    if not test -f "$config_file"
        touch "$config_file"
        or begin
            _forge_log error "Failed to create $config_file"
            return 1
        end
    end

    # Open editor with its own TTY session
    begin
        eval "$editor_cmd '$config_file'"
    end </dev/tty >/dev/tty 2>&1
    set -l exit_code $status

    if test $exit_code -ne 0
        _forge_log error "Editor exited with error code $exit_code"
    end

    _forge_reset
end

# Action handler: Show tools
function _forge_action_tools
    echo
    set -l agent_id "$_FORGE_ACTIVE_AGENT"
    if test -z "$agent_id"
        set agent_id forge
    end
    _forge_exec list tools "$agent_id"
end

# Action handler: Show skills
function _forge_action_skill
    echo
    _forge_exec list skill
end
