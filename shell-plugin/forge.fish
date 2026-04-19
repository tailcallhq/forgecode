# forge.fish — fish shell integration for ForgeCode
# ported from the zsh plugin (forge zsh plugin)

# --- state ---
set -g _FORGE_BIN (test -n "$FORGE_BIN"; and echo $FORGE_BIN; or command -v forge 2>/dev/null; or echo forge)
set -g _FORGE_CONVERSATION_ID ""
set -g _FORGE_PREVIOUS_CONVERSATION_ID ""
set -g _FORGE_ACTIVE_AGENT ""
set -g _FORGE_SESSION_MODEL ""
set -g _FORGE_SESSION_PROVIDER ""
set -g _FORGE_SESSION_REASONING_EFFORT ""
set -g _FORGE_COMMANDS ""
set -g _FORGE_MAX_COMMIT_DIFF (test -n "$FORGE_MAX_COMMIT_DIFF"; and echo $FORGE_MAX_COMMIT_DIFF; or echo 100000)
set -g _FORGE_DELIMITER '\s\s+'
set -g _FORGE_PREVIEW_WINDOW "--preview-window=bottom:75%:wrap:border-sharp"

# detect tools
if command -q bat
    set -g _FORGE_CAT_CMD "bat --color=always --style=numbers,changes --line-range=:500"
else
    set -g _FORGE_CAT_CMD cat
end

if command -q fdfind
    set -g _FORGE_FD_CMD fdfind
else if command -q fd
    set -g _FORGE_FD_CMD fd
else
    set -g _FORGE_FD_CMD fd
end


# --- helpers ---

function __forge_log
    set -l level $argv[1]
    set -l message $argv[2..-1]
    set -l ts (date '+%H:%M:%S')
    switch $level
        case error
            printf '\033[31m⏺\033[0m \033[90m[%s]\033[0m \033[31m%s\033[0m\n' $ts "$message"
        case info
            printf '\033[37m⏺\033[0m \033[90m[%s]\033[0m \033[37m%s\033[0m\n' $ts "$message"
        case success
            printf '\033[33m⏺\033[0m \033[90m[%s]\033[0m \033[37m%s\033[0m\n' $ts "$message"
        case warning
            printf '\033[93m⚠️\033[0m \033[90m[%s]\033[0m \033[93m%s\033[0m\n' $ts "$message"
        case debug
            printf '\033[36m⏺\033[0m \033[90m[%s]\033[0m \033[90m%s\033[0m\n' $ts "$message"
        case '*'
            echo $message
    end
end

function __forge_fzf
    fzf --reverse --exact --cycle --select-1 --height 80% --no-scrollbar --ansi --color="header:bold" $argv
end

function __forge_exec
    set -l agent_id (test -n "$_FORGE_ACTIVE_AGENT"; and echo $_FORGE_ACTIVE_AGENT; or echo forge)
    set -l cmd $_FORGE_BIN --agent $agent_id $argv
    # set -lx at function scope — if blocks create sub-scopes in fish 4.x
    test -n "$_FORGE_SESSION_MODEL"; and set -lx FORGE_SESSION__MODEL_ID "$_FORGE_SESSION_MODEL"
    test -n "$_FORGE_SESSION_PROVIDER"; and set -lx FORGE_SESSION__PROVIDER_ID "$_FORGE_SESSION_PROVIDER"
    test -n "$_FORGE_SESSION_REASONING_EFFORT"; and set -lx FORGE_REASONING__EFFORT "$_FORGE_SESSION_REASONING_EFFORT"
    $cmd
end

function __forge_exec_interactive
    set -l agent_id (test -n "$_FORGE_ACTIVE_AGENT"; and echo $_FORGE_ACTIVE_AGENT; or echo forge)
    set -l cmd $_FORGE_BIN --agent $agent_id $argv
    test -n "$_FORGE_SESSION_MODEL"; and set -lx FORGE_SESSION__MODEL_ID "$_FORGE_SESSION_MODEL"
    test -n "$_FORGE_SESSION_PROVIDER"; and set -lx FORGE_SESSION__PROVIDER_ID "$_FORGE_SESSION_PROVIDER"
    test -n "$_FORGE_SESSION_REASONING_EFFORT"; and set -lx FORGE_REASONING__EFFORT "$_FORGE_SESSION_REASONING_EFFORT"
    $cmd </dev/tty >/dev/tty
end

function __forge_get_commands
    if test -z "$_FORGE_COMMANDS"
        set -g _FORGE_COMMANDS (env CLICOLOR_FORCE=0 $_FORGE_BIN list commands --porcelain 2>/dev/null)
    end
    printf '%s\n' $_FORGE_COMMANDS
end

# stdin-based index finder: pipe lines in, pass value and field_num as args
# supports optional dual-field matching: __forge_find_index VALUE FIELD [FIELD2 VALUE2]
function __forge_find_index
    set -l value $argv[1]
    set -l field_num (test -n "$argv[2]"; and echo $argv[2]; or echo 1)
    set -l field_num2 ""
    set -l value2 ""
    if test (count $argv) -ge 4
        set field_num2 $argv[3]
        set value2 $argv[4]
    end
    set -l idx 1
    set -l line_num 0
    while read -l line
        set line_num (math $line_num + 1)
        if test $line_num -eq 1
            continue
        end
        set -l field_value (echo $line | awk -F '  +' "{print \$$field_num}")
        if test "$field_value" = "$value"
            if test -n "$field_num2"; and test -n "$value2"
                set -l field_value2 (echo $line | awk -F '  +' "{print \$$field_num2}")
                if test "$field_value2" = "$value2"
                    echo $idx
                    return 0
                end
            else
                echo $idx
                return 0
            end
        end
        set idx (math $idx + 1)
    end
    echo 1
end

function __forge_switch_conversation
    set -l new_id $argv[1]
    if test -n "$_FORGE_CONVERSATION_ID"; and test "$_FORGE_CONVERSATION_ID" != "$new_id"
        set -g _FORGE_PREVIOUS_CONVERSATION_ID $_FORGE_CONVERSATION_ID
    end
    set -g _FORGE_CONVERSATION_ID $new_id
end

function __forge_clear_conversation
    if test -n "$_FORGE_CONVERSATION_ID"
        set -g _FORGE_PREVIOUS_CONVERSATION_ID $_FORGE_CONVERSATION_ID
    end
    set -g _FORGE_CONVERSATION_ID ""
end

function __forge_start_background_sync
    set -l sync_enabled (test -n "$FORGE_SYNC_ENABLED"; and echo $FORGE_SYNC_ENABLED; or echo true)
    if test "$sync_enabled" != true
        return
    end
    set -l wp (pwd -P)
    set -l bin $_FORGE_BIN
    fish -c "
        if $bin workspace info '$wp' >/dev/null 2>&1
            $bin workspace sync '$wp' >/dev/null 2>&1
        end
    " &
    disown
end

function __forge_start_background_update
    set -l bin $_FORGE_BIN
    fish -c "$bin update --no-confirm >/dev/null 2>&1" &
    disown
end


# --- pick helpers (fzf wrappers) ---

function __forge_pick_model
    set -l prompt_text $argv[1]
    set -l current_model $argv[2]
    set -l input_text $argv[3]
    set -l current_provider $argv[4]
    set -l provider_field $argv[5]
    set -l output ($_FORGE_BIN list models --porcelain 2>/dev/null)
    if test -z "$output"
        return 1
    end
    set -l fzf_args --delimiter="$_FORGE_DELIMITER" --prompt="$prompt_text" --with-nth="2,3,5.."
    if test -n "$input_text"
        set -a fzf_args --query="$input_text"
    end
    if test -n "$current_model"
        if test -n "$current_provider"; and test -n "$provider_field"
            set -l idx (printf '%s\n' $output | __forge_find_index "$current_model" 1 "$provider_field" "$current_provider")
            set -a fzf_args --bind="start:pos($idx)"
        else
            set -l idx (printf '%s\n' $output | __forge_find_index "$current_model" 1)
            set -a fzf_args --bind="start:pos($idx)"
        end
    end
    printf '%s\n' $output | __forge_fzf --header-lines=1 $fzf_args
end

function __forge_select_provider
    set -l filter_status $argv[1]
    set -l current_provider $argv[2]
    set -l filter_type $argv[3]
    set -l query $argv[4]
    set -l cmd $_FORGE_BIN list provider --porcelain
    if test -n "$filter_type"
        set cmd $cmd --type=$filter_type
    end
    set -l output ($cmd 2>/dev/null)
    if test -z "$output"
        __forge_log error "No providers available"
        return 1
    end
    if test -n "$filter_status"
        set -l header $output[1]
        set -l filtered (printf '%s\n' $output[2..-1] | grep -i "$filter_status")
        if test -z "$filtered"
            __forge_log error "No $filter_status providers found"
            return 1
        end
        set output $header $filtered
    end
    if test -z "$current_provider"
        set current_provider ($_FORGE_BIN config get provider --porcelain 2>/dev/null)
    end
    set -l fzf_args --delimiter="$_FORGE_DELIMITER" --prompt="Provider ❯ " --with-nth=1,3..
    if test -n "$query"
        set -a fzf_args --query="$query"
    end
    if test -n "$current_provider"
        set -l idx (printf '%s\n' $output | __forge_find_index "$current_provider" 1)
        set -a fzf_args --bind="start:pos($idx)"
    end
    printf '%s\n' $output | __forge_fzf --header-lines=1 $fzf_args
end


# --- action functions ---

function __forge_action_new
    set -l input_text $argv[1]
    __forge_clear_conversation
    set -g _FORGE_ACTIVE_AGENT forge
    echo
    if test -n "$input_text"
        set -l new_id ($_FORGE_BIN conversation new)
        __forge_switch_conversation $new_id
        __forge_exec_interactive -p "$input_text" --cid $_FORGE_CONVERSATION_ID
        __forge_start_background_sync
        __forge_start_background_update
    else
        __forge_exec banner
    end
end

function __forge_action_info
    echo
    if test -n "$_FORGE_CONVERSATION_ID"
        __forge_exec info --cid $_FORGE_CONVERSATION_ID
    else
        __forge_exec info
    end
end

function __forge_action_env
    echo
    __forge_exec env
end

function __forge_action_agent
    set -l input_text $argv[1]
    echo
    if test -n "$input_text"
        set -l agent_exists ($_FORGE_BIN list agents --porcelain 2>/dev/null | tail -n +2 | grep -q "^$input_text\\b"; and echo true; or echo false)
        if test "$agent_exists" = false
            __forge_log error "Agent '\033[1m$input_text\033[0m' not found"
            return
        end
        set -g _FORGE_ACTIVE_AGENT $input_text
        __forge_log success "Switched to agent \033[1m$input_text\033[0m"
        return
    end
    set -l agents_output ($_FORGE_BIN list agents --porcelain 2>/dev/null)
    if test -n "$agents_output"
        set -l fzf_args --prompt="Agent ❯ " --delimiter="$_FORGE_DELIMITER" --with-nth="1,2,4,5,6"
        if test -n "$_FORGE_ACTIVE_AGENT"
            set -l idx (printf '%s\n' $agents_output | __forge_find_index "$_FORGE_ACTIVE_AGENT")
            set -a fzf_args --bind="start:pos($idx)"
        end
        set -l selected (printf '%s\n' $agents_output | __forge_fzf --header-lines=1 $fzf_args)
        if test -n "$selected"
            set -l agent_id (echo $selected | awk '{print $1}')
            set -g _FORGE_ACTIVE_AGENT $agent_id
            __forge_log success "Switched to agent \033[1m$agent_id\033[0m"
        end
    else
        __forge_log error "No agents found"
    end
end

function __forge_action_model
    set -l input_text $argv[1]
    echo
    set -l current_model ($_FORGE_BIN config get model 2>/dev/null)
    set -l current_provider ($_FORGE_BIN config get provider 2>/dev/null)
    set -l selected (__forge_pick_model "Model ❯ " "$current_model" "$input_text" "$current_provider" 3)
    if test -n "$selected"
        set -l model_id (echo $selected | awk -F '  +' '{print $1}' | string trim)
        set -l provider_display (echo $selected | awk -F '  +' '{print $3}' | string trim)
        set -l provider_id (echo $selected | awk -F '  +' '{print $4}' | string trim)
        if test -n "$provider_display"; and test "$provider_display" != "$current_provider"
            __forge_exec_interactive config set provider $provider_id --model $model_id
            return
        end
        __forge_exec config set model $model_id
    end
end

function __forge_action_session_model
    set -l input_text $argv[1]
    echo
    set -l current_model
    set -l current_provider
    set -l provider_index
    if test -n "$_FORGE_SESSION_MODEL"
        set current_model $_FORGE_SESSION_MODEL
        set provider_index 4
    else
        set current_model ($_FORGE_BIN config get model 2>/dev/null)
        set provider_index 3
    end
    if test -n "$_FORGE_SESSION_PROVIDER"
        set current_provider $_FORGE_SESSION_PROVIDER
        set provider_index 4
    else
        set current_provider ($_FORGE_BIN config get provider 2>/dev/null)
        set provider_index 3
    end
    set -l selected (__forge_pick_model "Session Model ❯ " "$current_model" "$input_text" "$current_provider" "$provider_index")
    if test -n "$selected"
        set -l model_id (echo $selected | awk -F '  +' '{print $1}' | string trim)
        set -l provider_id (echo $selected | awk -F '  +' '{print $4}' | string trim)
        set -g _FORGE_SESSION_MODEL $model_id
        set -g _FORGE_SESSION_PROVIDER $provider_id
        __forge_log success "Session model set to \033[1m$model_id\033[0m (provider: \033[1m$provider_id\033[0m)"
    end
end

function __forge_action_config_reload
    echo
    if test -z "$_FORGE_SESSION_MODEL"; and test -z "$_FORGE_SESSION_PROVIDER"; and test -z "$_FORGE_SESSION_REASONING_EFFORT"
        __forge_log info "No session overrides active (already using global config)"
        return
    end
    set -g _FORGE_SESSION_MODEL ""
    set -g _FORGE_SESSION_PROVIDER ""
    set -g _FORGE_SESSION_REASONING_EFFORT ""
    __forge_log success "Session overrides cleared — using global config"
end

function __forge_action_reasoning_effort
    set -l input_text $argv[1]
    echo
    set -l efforts "EFFORT" none minimal low medium high xhigh max
    set -l current_effort
    if test -n "$_FORGE_SESSION_REASONING_EFFORT"
        set current_effort $_FORGE_SESSION_REASONING_EFFORT
    else
        set current_effort ($_FORGE_BIN config get reasoning-effort 2>/dev/null)
    end
    set -l fzf_args --prompt="Reasoning Effort ❯ "
    if test -n "$input_text"
        set -a fzf_args --query="$input_text"
    end
    if test -n "$current_effort"
        set -l idx (printf '%s\n' $efforts | __forge_find_index "$current_effort" 1)
        set -a fzf_args --bind="start:pos($idx)"
    end
    set -l selected (printf '%s\n' $efforts | __forge_fzf --header-lines=1 $fzf_args)
    if test -n "$selected"
        set -g _FORGE_SESSION_REASONING_EFFORT $selected
        __forge_log success "Session reasoning effort set to \033[1m$selected\033[0m"
    end
end

function __forge_action_config_reasoning_effort
    set -l input_text $argv[1]
    echo
    set -l efforts "EFFORT" none minimal low medium high xhigh max
    set -l current_effort ($_FORGE_BIN config get reasoning-effort 2>/dev/null)
    set -l fzf_args --prompt="Config Reasoning Effort ❯ "
    if test -n "$input_text"
        set -a fzf_args --query="$input_text"
    end
    if test -n "$current_effort"
        set -l idx (printf '%s\n' $efforts | __forge_find_index "$current_effort" 1)
        set -a fzf_args --bind="start:pos($idx)"
    end
    set -l selected (printf '%s\n' $efforts | __forge_fzf --header-lines=1 $fzf_args)
    if test -n "$selected"
        __forge_exec config set reasoning-effort $selected
    end
end

function __forge_action_commit_model
    set -l input_text $argv[1]
    echo
    set -l commit_output (__forge_exec config get commit 2>/dev/null)
    set -l current_commit_provider $commit_output[1]
    set -l current_commit_model $commit_output[-1]
    set -l selected (__forge_pick_model "Commit Model ❯ " "$current_commit_model" "$input_text" "$current_commit_provider" 4)
    if test -n "$selected"
        set -l model_id (echo $selected | awk -F '  +' '{print $1}' | string trim)
        set -l provider_id (echo $selected | awk -F '  +' '{print $4}' | string trim)
        __forge_exec config set commit $provider_id $model_id
    end
end

function __forge_action_suggest_model
    set -l input_text $argv[1]
    echo
    set -l suggest_output (__forge_exec config get suggest 2>/dev/null)
    set -l current_suggest_provider $suggest_output[1]
    set -l current_suggest_model $suggest_output[-1]
    set -l selected (__forge_pick_model "Suggest Model ❯ " "$current_suggest_model" "$input_text" "$current_suggest_provider" 4)
    if test -n "$selected"
        set -l model_id (echo $selected | awk -F '  +' '{print $1}' | string trim)
        set -l provider_id (echo $selected | awk -F '  +' '{print $4}' | string trim)
        __forge_exec config set suggest $provider_id $model_id
    end
end

function __forge_action_conversation
    set -l input_text $argv[1]
    echo
    if test "$input_text" = "-"
        if test -z "$_FORGE_PREVIOUS_CONVERSATION_ID"
            set input_text ""
        else
            set -l temp $_FORGE_CONVERSATION_ID
            set -g _FORGE_CONVERSATION_ID $_FORGE_PREVIOUS_CONVERSATION_ID
            set -g _FORGE_PREVIOUS_CONVERSATION_ID $temp
            echo
            __forge_exec conversation show $_FORGE_CONVERSATION_ID
            __forge_exec conversation info $_FORGE_CONVERSATION_ID
            __forge_log success "Switched to conversation \033[1m$_FORGE_CONVERSATION_ID\033[0m"
            return
        end
    end
    if test -n "$input_text"
        __forge_switch_conversation $input_text
        echo
        __forge_exec conversation show $input_text
        __forge_exec conversation info $input_text
        __forge_log success "Switched to conversation \033[1m$input_text\033[0m"
        return
    end
    set -l conversations_output ($_FORGE_BIN conversation list --porcelain 2>/dev/null)
    if test -n "$conversations_output"
        set -l fzf_args --prompt="Conversation ❯ " --delimiter="$_FORGE_DELIMITER" --with-nth="2,3" --preview="env CLICOLOR_FORCE=1 $_FORGE_BIN conversation info {1}; echo; env CLICOLOR_FORCE=1 $_FORGE_BIN conversation show {1}" $_FORGE_PREVIEW_WINDOW
        if test -n "$_FORGE_CONVERSATION_ID"
            set -l idx (printf '%s\n' $conversations_output | __forge_find_index "$_FORGE_CONVERSATION_ID" 1)
            set -a fzf_args --bind="start:pos($idx)"
        end
        set -l selected (printf '%s\n' $conversations_output | __forge_fzf --header-lines=1 $fzf_args)
        if test -n "$selected"
            set -l cid (echo $selected | string replace -r '  .*' '' | string trim)
            __forge_switch_conversation $cid
            echo
            __forge_exec conversation show $cid
            __forge_exec conversation info $cid
            __forge_log success "Switched to conversation \033[1m$cid\033[0m"
        end
    else
        __forge_log error "No conversations found"
    end
end

function __forge_action_clone
    set -l input_text $argv[1]
    echo
    if test -n "$input_text"
        __forge_clone_and_switch $input_text
        return
    end
    set -l conversations_output ($_FORGE_BIN conversation list --porcelain 2>/dev/null)
    if test -z "$conversations_output"
        __forge_log error "No conversations found"
        return
    end
    set -l fzf_args --prompt="Clone Conversation ❯ " --delimiter="$_FORGE_DELIMITER" --with-nth="2,3" --preview="env CLICOLOR_FORCE=1 $_FORGE_BIN conversation info {1}; echo; env CLICOLOR_FORCE=1 $_FORGE_BIN conversation show {1}" $_FORGE_PREVIEW_WINDOW
    if test -n "$_FORGE_CONVERSATION_ID"
        set -l idx (printf '%s\n' $conversations_output | __forge_find_index "$_FORGE_CONVERSATION_ID")
        set -a fzf_args --bind="start:pos($idx)"
    end
    set -l selected (printf '%s\n' $conversations_output | __forge_fzf --header-lines=1 $fzf_args)
    if test -n "$selected"
        set -l cid (echo $selected | string replace -r '  .*' '' | string trim)
        __forge_clone_and_switch $cid
    end
end

function __forge_clone_and_switch
    set -l clone_target $argv[1]
    set -l original $_FORGE_CONVERSATION_ID
    __forge_log info "Cloning conversation \033[1m$clone_target\033[0m"
    set -l clone_output ($_FORGE_BIN conversation clone $clone_target 2>&1)
    if test $status -eq 0
        set -l new_id (printf '%s\n' $clone_output | grep -oE '[a-f0-9-]{36}' | tail -1)
        if test -n "$new_id"
            __forge_switch_conversation $new_id
            __forge_log success "└─ Switched to conversation \033[1m$new_id\033[0m"
            if test "$clone_target" != "$original"
                echo
                __forge_exec conversation show $new_id
                echo
                __forge_exec conversation info $new_id
            end
        else
            __forge_log error "Failed to extract conversation ID from clone output"
        end
    else
        __forge_log error "Failed to clone conversation: $clone_output"
    end
end

function __forge_action_copy
    echo
    if test -z "$_FORGE_CONVERSATION_ID"
        __forge_log error "No active conversation"
        return
    end
    set -l content ($_FORGE_BIN conversation show --md $_FORGE_CONVERSATION_ID 2>/dev/null)
    if test -z "$content"
        __forge_log error "No assistant message found"
        return
    end
    set -l joined (string join \n -- $content)
    if command -q pbcopy
        printf '%s' "$joined" | pbcopy
    else if command -q xclip
        printf '%s' "$joined" | xclip -selection clipboard
    else if command -q xsel
        printf '%s' "$joined" | xsel --clipboard --input
    else
        __forge_log error "No clipboard utility found"
        return
    end
    set -l lc (count $content)
    set -l bc (string length -- "$joined")
    __forge_log success "Copied to clipboard \033[90m[$lc lines, $bc bytes]\033[0m"
end

function __forge_action_rename
    set -l input_text $argv[1]
    echo
    if test -z "$_FORGE_CONVERSATION_ID"
        __forge_log error "No active conversation"
        return
    end
    if test -z "$input_text"
        __forge_log error "Usage: :rename <name>"
        return
    end
    __forge_exec conversation rename $_FORGE_CONVERSATION_ID $input_text
end

function __forge_action_conversation_rename
    set -l input_text $argv[1]
    echo
    if test -n "$input_text"
        set -l conversation_id (string split ' ' -- $input_text)[1]
        set -l new_name (string replace -r '^\S+\s+' '' -- "$input_text")
        if test "$conversation_id" = "$new_name"
            __forge_log error "Usage: :conversation-rename <id> <name>"
            return
        end
        __forge_exec conversation rename $conversation_id $new_name
        return
    end
    set -l conversations_output ($_FORGE_BIN conversation list --porcelain 2>/dev/null)
    if test -z "$conversations_output"
        __forge_log error "No conversations found"
        return
    end
    set -l fzf_args --prompt="Rename Conversation ❯ " --delimiter="$_FORGE_DELIMITER" --with-nth="2,3" --preview="env CLICOLOR_FORCE=1 $_FORGE_BIN conversation info {1}; echo; env CLICOLOR_FORCE=1 $_FORGE_BIN conversation show {1}" $_FORGE_PREVIEW_WINDOW
    if test -n "$_FORGE_CONVERSATION_ID"
        set -l idx (printf '%s\n' $conversations_output | __forge_find_index "$_FORGE_CONVERSATION_ID" 1)
        set -a fzf_args --bind="start:pos($idx)"
    end
    set -l selected (printf '%s\n' $conversations_output | __forge_fzf --header-lines=1 $fzf_args)
    if test -n "$selected"
        set -l cid (echo $selected | string replace -r '  .*' '' | string trim)
        read -P "Enter new name: " new_name </dev/tty
        if test -n "$new_name"
            __forge_exec conversation rename $cid $new_name
        else
            __forge_log error "No name provided, rename cancelled"
        end
    end
end

function __forge_action_tools
    echo
    set -l agent_id (test -n "$_FORGE_ACTIVE_AGENT"; and echo $_FORGE_ACTIVE_AGENT; or echo forge)
    __forge_exec list tools $agent_id
end

function __forge_action_skill
    echo
    __forge_exec list skill
end

function __forge_action_config
    echo
    $_FORGE_BIN config list
end

function __forge_action_config_edit
    echo
    set -l editor_cmd (if test -n "$FORGE_EDITOR"; echo $FORGE_EDITOR; else if test -n "$EDITOR"; echo $EDITOR; else; echo nano; end)
    set -l editor_bin (string split ' ' -- $editor_cmd)[1]
    if not command -q $editor_bin
        __forge_log error "Editor not found: $editor_cmd (set FORGE_EDITOR or EDITOR)"
        return 1
    end
    set -l config_file "$HOME/forge/.forge.toml"
    if not test -d "$HOME/forge"
        mkdir -p "$HOME/forge"
    end
    if not test -f "$config_file"
        touch "$config_file"
    end
    eval $editor_cmd "'$config_file'" </dev/tty >/dev/tty 2>&1
    set -l exit_code $status
    if test $exit_code -ne 0
        __forge_log error "Editor exited with error code $exit_code"
    end
    set -g _FORGE_COMMANDS ""
end

function __forge_action_sync
    echo
    __forge_exec_interactive workspace sync --init
end

function __forge_action_sync_init
    echo
    __forge_exec_interactive workspace init
end

function __forge_action_sync_status
    echo
    __forge_exec workspace status "."
end

function __forge_action_sync_info
    echo
    __forge_exec workspace info "."
end

function __forge_action_login
    set -l input_text $argv[1]
    echo
    set -l selected (__forge_select_provider "" "" "" "$input_text")
    if test -n "$selected"
        set -l provider (echo $selected | awk '{print $2}')
        __forge_exec_interactive provider login $provider
    end
end

function __forge_action_logout
    set -l input_text $argv[1]
    echo
    set -l selected (__forge_select_provider '\\[yes\\]' "" "" "$input_text")
    if test -n "$selected"
        set -l provider (echo $selected | awk '{print $2}')
        __forge_exec provider logout $provider
    end
end

function __forge_action_suggest
    set -l description $argv[1]
    if test -z "$description"
        __forge_log error "Please provide a command description"
        return
    end
    echo
    set -lx FORCE_COLOR true
    set -lx CLICOLOR_FORCE 1
    set -l generated_command (__forge_exec suggest "$description" | string collect)
    if test -n "$generated_command"
        commandline -r "$generated_command"
        commandline -f repaint
    else
        __forge_log error "Failed to generate command"
    end
end

function __forge_action_commit
    set -l additional_context $argv[1]
    echo
    if test -n "$additional_context"
        env FORCE_COLOR=true CLICOLOR_FORCE=1 $_FORGE_BIN commit --max-diff $_FORGE_MAX_COMMIT_DIFF $additional_context
    else
        env FORCE_COLOR=true CLICOLOR_FORCE=1 $_FORGE_BIN commit --max-diff $_FORGE_MAX_COMMIT_DIFF
    end
end

function __forge_action_commit_preview
    set -l additional_context $argv[1]
    echo
    set -l commit_message
    if test -n "$additional_context"
        set commit_message (env FORCE_COLOR=true CLICOLOR_FORCE=1 $_FORGE_BIN commit --preview --max-diff $_FORGE_MAX_COMMIT_DIFF $additional_context | string collect)
    else
        set commit_message (env FORCE_COLOR=true CLICOLOR_FORCE=1 $_FORGE_BIN commit --preview --max-diff $_FORGE_MAX_COMMIT_DIFF | string collect)
    end
    if test -n "$commit_message"
        # single-quote wrap with internal quote escaping (matches zsh ${(qq)})
        set -l quoted (string replace -a "'" "'\\''" -- "$commit_message")
        if git diff --staged --quiet
            commandline -r "git commit -am '$quoted'"
        else
            commandline -r "git commit -m '$quoted'"
        end
        commandline -f repaint
    end
end

function __forge_action_editor
    set -l initial_text $argv[1]
    echo
    set -l editor_cmd (if test -n "$FORGE_EDITOR"; echo $FORGE_EDITOR; else if test -n "$EDITOR"; echo $EDITOR; else; echo nano; end)
    set -l editor_bin (string split ' ' -- $editor_cmd)[1]
    if not command -q $editor_bin
        __forge_log error "Editor not found: $editor_cmd (set FORGE_EDITOR or EDITOR)"
        return 1
    end
    set -l forge_dir .forge
    if not test -d $forge_dir
        mkdir -p $forge_dir
    end
    set -l temp_file $forge_dir/FORGE_EDITMSG.md
    if test -n "$initial_text"
        echo "$initial_text" >$temp_file
    else
        echo -n "" >$temp_file
    end
    eval $editor_cmd "'$temp_file'" </dev/tty >/dev/tty 2>&1
    set -l editor_status $status
    if test $editor_status -ne 0
        __forge_log error "Editor exited with error code $editor_status"
        rm -f $temp_file 2>/dev/null
        return 1
    end
    set -l content (cat $temp_file | tr -d '\r' | string collect)
    rm -f $temp_file 2>/dev/null
    if test -z "$content"
        __forge_log info "Editor closed with no content"
        return
    end
    commandline -r ": $content"
    commandline -f repaint
end

function __forge_action_dump
    set -l input_text $argv[1]
    echo
    if test -z "$_FORGE_CONVERSATION_ID"
        __forge_log error "No active conversation"
        return
    end
    if test "$input_text" = html
        __forge_exec conversation dump $_FORGE_CONVERSATION_ID --html
    else
        __forge_exec conversation dump $_FORGE_CONVERSATION_ID
    end
end

function __forge_action_compact
    echo
    if test -z "$_FORGE_CONVERSATION_ID"
        __forge_log error "No active conversation"
        return
    end
    __forge_exec conversation compact $_FORGE_CONVERSATION_ID
end

function __forge_action_retry
    echo
    if test -z "$_FORGE_CONVERSATION_ID"
        __forge_log error "No active conversation"
        return
    end
    __forge_exec conversation retry $_FORGE_CONVERSATION_ID
end

function __forge_action_doctor
    echo
    $_FORGE_BIN zsh doctor
end

function __forge_action_keyboard
    echo
    $_FORGE_BIN zsh keyboard
end

function __forge_action_default
    set -l user_action $argv[1]
    set -l input_text $argv[2]
    set -l command_type ""
    if test -n "$user_action"
        set -l commands_list (__forge_get_commands)
        if test -n "$commands_list"
            set -l command_row (printf '%s\n' $commands_list | grep -m1 "^$user_action  ")
            if test -z "$command_row"
                echo
                __forge_log error "Command '\033[1m$user_action\033[0m' not found"
                return
            end
            set command_type (echo $command_row | awk '{print $2}')
            if test (string lower "$command_type") = custom
                if test -z "$_FORGE_CONVERSATION_ID"
                    set -g _FORGE_CONVERSATION_ID ($_FORGE_BIN conversation new)
                end
                echo
                if test -n "$input_text"
                    __forge_exec cmd execute --cid $_FORGE_CONVERSATION_ID $user_action "$input_text"
                else
                    __forge_exec cmd execute --cid $_FORGE_CONVERSATION_ID $user_action
                end
                return
            end
        end
    end
    if test -z "$input_text"
        if test -n "$user_action"
            if test (string lower "$command_type") != agent
                echo
                __forge_log error "Command '\033[1m$user_action\033[0m' not found"
                return
            end
            echo
            set -g _FORGE_ACTIVE_AGENT $user_action
            __forge_log info "\033[1;37m"(string upper $_FORGE_ACTIVE_AGENT)"\033[0m \033[90mis now the active agent\033[0m"
        end
        return
    end
    if test -z "$_FORGE_CONVERSATION_ID"
        set -g _FORGE_CONVERSATION_ID ($_FORGE_BIN conversation new)
    end
    echo
    if test -n "$user_action"
        set -g _FORGE_ACTIVE_AGENT $user_action
    end
    __forge_exec_interactive -p "$input_text" --cid $_FORGE_CONVERSATION_ID
    __forge_start_background_sync
    __forge_start_background_update
end


# --- main dispatcher (the enter key handler) ---

function __forge_accept_line
    set -l buf (commandline)

    # check if the line starts with our sentinel
    if not string match -qr '^:' -- "$buf"
        # not a forge command — execute normally
        commandline -f execute
        return
    end

    # parse the command: ":action rest" or ": rest"
    set -l user_action ""
    set -l input_text ""
    set -l parts

    if set parts (string match -r '^:([a-zA-Z][a-zA-Z0-9_-]*)\s+(.+)$' -- "$buf")
        set user_action $parts[2]
        set input_text $parts[3]
    else if set parts (string match -r '^:([a-zA-Z][a-zA-Z0-9_-]*)$' -- "$buf")
        set user_action $parts[2]
    else if set parts (string match -r '^: (.+)$' -- "$buf")
        set input_text $parts[2]
    else if test "$buf" = ":"
        commandline -r ""
        return
    else
        commandline -f execute
        return
    end

    # add to history — fish has no builtin history-add command,
    # so we write directly to the history file and merge
    set -l hist_file (if set -q __fish_user_data_dir; echo $__fish_user_data_dir; else; echo ~/.local/share/fish; end)/fish_history
    set -l escaped_buf (string replace -a '\\' '\\\\' -- "$buf" | string replace -a '\n' '\\n')
    printf '- cmd: %s\n  when: %d\n' "$escaped_buf" (date +%s) >> "$hist_file"
    builtin history merge

    # clear the commandline before running action
    commandline -r ""

    # aliases
    switch "$user_action"
        case ask
            set user_action sage
        case plan
            set user_action muse
    end

    # route to action
    switch "$user_action"
        case new n
            __forge_action_new "$input_text"
        case info i
            __forge_action_info
        case env e
            __forge_action_env
        case dump d
            __forge_action_dump "$input_text"
        case compact
            __forge_action_compact
        case retry r
            __forge_action_retry
        case agent a
            __forge_action_agent "$input_text"
        case conversation c
            __forge_action_conversation "$input_text"
        case config-model cm
            __forge_action_model "$input_text"
        case model m
            __forge_action_session_model "$input_text"
        case config-reload cr model-reset mr
            __forge_action_config_reload
        case reasoning-effort re
            __forge_action_reasoning_effort "$input_text"
        case config-reasoning-effort cre
            __forge_action_config_reasoning_effort "$input_text"
        case config-commit-model ccm
            __forge_action_commit_model "$input_text"
        case config-suggest-model csm
            __forge_action_suggest_model "$input_text"
        case tools t
            __forge_action_tools
        case config
            __forge_action_config
        case config-edit ce
            __forge_action_config_edit
        case skill
            __forge_action_skill
        case edit ed
            __forge_action_editor "$input_text"
        case commit
            __forge_action_commit "$input_text"
        case commit-preview
            __forge_action_commit_preview "$input_text"
        case suggest s
            __forge_action_suggest "$input_text"
        case clone
            __forge_action_clone "$input_text"
        case rename rn
            __forge_action_rename "$input_text"
        case conversation-rename
            __forge_action_conversation_rename "$input_text"
        case copy
            __forge_action_copy
        case workspace-sync sync
            __forge_action_sync
        case workspace-init sync-init
            __forge_action_sync_init
        case workspace-status sync-status
            __forge_action_sync_status
        case workspace-info sync-info
            __forge_action_sync_info
        case provider-login login provider
            __forge_action_login "$input_text"
        case logout
            __forge_action_logout "$input_text"
        case doctor
            __forge_action_doctor
        case keyboard-shortcuts kb
            __forge_action_keyboard
        case '*'
            __forge_action_default "$user_action" "$input_text"
    end

    commandline -f repaint
end


# --- tab completion handler ---

function __forge_completion
    set -l buf (commandline)

    # get the word under cursor
    set -l current_token (commandline -ct)

    # @ file tagging
    if string match -qr '^@' -- "$current_token"
        set -l filter_text (string replace '@' '' -- "$current_token")
        set -l fzf_args --preview="if [ -d {} ]; then ls -la --color=always {} 2>/dev/null || ls -la {}; else $_FORGE_CAT_CMD {}; fi" $_FORGE_PREVIEW_WINDOW
        set -l file_list ($_FORGE_FD_CMD --type f --type d --hidden --exclude .git)
        set -l selected
        if test -n "$filter_text"
            set selected (printf '%s\n' $file_list | __forge_fzf --query "$filter_text" $fzf_args)
        else
            set selected (printf '%s\n' $file_list | __forge_fzf $fzf_args)
        end
        if test -n "$selected"
            commandline -rt "@[$selected]"
        end
        commandline -f repaint
        return
    end

    # :command completion (only when line is just a colon prefix)
    if string match -qr '^:[a-zA-Z][a-zA-Z0-9_-]*$' -- "$buf"; or test "$buf" = ":"
        set -l filter_text (string replace ':' '' -- "$buf")
        set -l commands_list (__forge_get_commands)
        if test -n "$commands_list"
            set -l selected
            if test -n "$filter_text"
                set selected (printf '%s\n' $commands_list | __forge_fzf --header-lines=1 --delimiter="$_FORGE_DELIMITER" --nth=1 --query "$filter_text" --prompt="Command ❯ ")
            else
                set selected (printf '%s\n' $commands_list | __forge_fzf --header-lines=1 --delimiter="$_FORGE_DELIMITER" --nth=1 --prompt="Command ❯ ")
            end
            if test -n "$selected"
                set -l command_name (echo $selected | awk '{print $1}')
                commandline -r ":$command_name "
            end
        end
        commandline -f repaint
        return
    end

    # default: normal tab completion
    commandline -f complete
end


# --- right prompt ---

function __forge_rprompt
    set -lx _FORGE_CONVERSATION_ID "$_FORGE_CONVERSATION_ID"
    set -lx _FORGE_ACTIVE_AGENT "$_FORGE_ACTIVE_AGENT"
    test -n "$_FORGE_SESSION_MODEL"; and set -lx FORGE_SESSION__MODEL_ID "$_FORGE_SESSION_MODEL"
    test -n "$_FORGE_SESSION_PROVIDER"; and set -lx FORGE_SESSION__PROVIDER_ID "$_FORGE_SESSION_PROVIDER"
    test -n "$_FORGE_SESSION_REASONING_EFFORT"; and set -lx FORGE_REASONING__EFFORT "$_FORGE_SESSION_REASONING_EFFORT"
    $_FORGE_BIN zsh rprompt 2>/dev/null
end

# inject into right prompt — only if we haven't already
if not set -q _FORGE_FISH_LOADED
    if functions -q fish_right_prompt
        functions -c fish_right_prompt __forge_original_right_prompt
        function fish_right_prompt
            set -l forge_info (__forge_rprompt)
            set -l original (__forge_original_right_prompt 2>/dev/null)
            if test -n "$forge_info"
                if test -n "$original"
                    echo "$forge_info $original"
                else
                    echo "$forge_info"
                end
            else if test -n "$original"
                echo "$original"
            end
        end
    else
        function fish_right_prompt
            __forge_rprompt
        end
    end
    set -g _FORGE_FISH_LOADED (date +%s)
end


# --- key bindings ---

bind \r __forge_accept_line
bind \n __forge_accept_line
bind \t __forge_completion

# vi mode support
if bind -M insert \r 2>/dev/null
    bind -M insert \r __forge_accept_line
    bind -M insert \n __forge_accept_line
    bind -M insert \t __forge_completion
end
