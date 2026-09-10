# Fish completions for forge CLI
# Auto-loaded from ~/.config/fish/completions/

# ── Helpers ───────────────────────────────────────────────────────────────────
# __forge_needs_subcommand <parent>
#   True when <parent> has been seen but the next positional arg is still needed.
#   Works by counting positional (non-option) tokens on the command line.
function __forge_needs_subcommand
    set -l parent $argv[1]
    set -l cmd (commandline -opc)
    set -l found_parent 0
    for tok in $cmd[2..]   # skip "forge" itself
        switch $tok
            case '-*'
                continue
            case '*'
                if test $found_parent -eq 0
                    if test "$tok" = "$parent"
                        set found_parent 1
                    end
                else
                    # A positional token after parent => subcommand already present
                    return 1
                end
        end
    end
    # Return true only if we found the parent and there's no sub yet
    test $found_parent -eq 1
end

# __forge_at_depth3 <parent> <child>
#   True when "forge <parent> <child>" have been seen and a 3rd positional
#   argument is still needed (e.g. "forge config set <TAB>").
function __forge_at_depth3
    set -l parent $argv[1]
    set -l child  $argv[2]
    set -l cmd (commandline -opc)
    set -l depth 0
    set -l match_parent 0
    set -l match_child 0
    for tok in $cmd[2..]
        switch $tok
            case '-*'
                continue
            case '*'
                set depth (math $depth + 1)
                if test $depth -eq 1; and test "$tok" = "$parent"
                    set match_parent 1
                else if test $depth -eq 2; and test $match_parent -eq 1; and test "$tok" = "$child"
                    set match_child 1
                else if test $depth -ge 3
                    return 1  # already have a 3rd token
                end
        end
    end
    test $match_parent -eq 1; and test $match_child -eq 1
end

# Disable file completions by default for forge
complete -c forge -f

# ── Top-level subcommands ─────────────────────────────────────────────────────
complete -c forge -n __fish_use_subcommand -a agent -d "Manage agents"
complete -c forge -n __fish_use_subcommand -a zsh -d "Generate shell extension scripts"
complete -c forge -n __fish_use_subcommand -a list -d "List agents, models, providers, tools, or MCP servers"
complete -c forge -n __fish_use_subcommand -a banner -d "Display the banner with version information"
complete -c forge -n __fish_use_subcommand -a info -d "Show configuration, active model, and environment status"
complete -c forge -n __fish_use_subcommand -a env -d "Display environment information"
complete -c forge -n __fish_use_subcommand -a config -d "Get, set, or list configuration values"
complete -c forge -n __fish_use_subcommand -a conversation -d "Manage conversation history and state"
complete -c forge -n __fish_use_subcommand -a commit -d "Generate and optionally commit changes with AI-generated message"
complete -c forge -n __fish_use_subcommand -a mcp -d "Manage Model Context Protocol servers"
complete -c forge -n __fish_use_subcommand -a suggest -d "Generate shell commands without executing them"
complete -c forge -n __fish_use_subcommand -a provider -d "Manage API provider authentication"
complete -c forge -n __fish_use_subcommand -a cmd -d "Run or list custom commands"
complete -c forge -n __fish_use_subcommand -a workspace -d "Manage workspaces for semantic search"
complete -c forge -n __fish_use_subcommand -a data -d "Process JSONL data through LLM"
complete -c forge -n __fish_use_subcommand -a vscode -d "VS Code integration commands"
complete -c forge -n __fish_use_subcommand -a update -d "Update forge to the latest version"
complete -c forge -n __fish_use_subcommand -a setup -d "Setup zsh integration"
complete -c forge -n __fish_use_subcommand -a doctor -d "Run diagnostics on shell environment"

# Top-level options
complete -c forge -n __fish_use_subcommand -s p -l prompt -d "Direct prompt to process"
complete -c forge -n __fish_use_subcommand -l conversation -d "Path to conversation JSON" -r -F
complete -c forge -n __fish_use_subcommand -l conversation-id -d "Conversation ID to use"
complete -c forge -n __fish_use_subcommand -s C -l directory -d "Working directory" -r -a "(__fish_complete_directories)"
complete -c forge -n __fish_use_subcommand -l sandbox -d "Isolated git worktree name"
complete -c forge -n __fish_use_subcommand -l verbose -d "Enable verbose logging"
complete -c forge -n __fish_use_subcommand -l agent -d "Agent ID to use"
complete -c forge -n __fish_use_subcommand -s e -l event -d "Event to dispatch (JSON)"
complete -c forge -n __fish_use_subcommand -s h -l help -d "Print help"
complete -c forge -n __fish_use_subcommand -s V -l version -d "Print version"

# ── config subcommands ────────────────────────────────────────────────────────
complete -c forge -n "__forge_needs_subcommand config" -a set -d "Set a configuration value"
complete -c forge -n "__forge_needs_subcommand config" -a get -d "Get a configuration value"
complete -c forge -n "__forge_needs_subcommand config" -a list -d "List configuration"

# config set targets
complete -c forge -n "__forge_at_depth3 config set" -a model -d "Set default model"
complete -c forge -n "__forge_at_depth3 config set" -a provider -d "Set default provider"
complete -c forge -n "__forge_at_depth3 config set" -a commit -d "Set commit model"
complete -c forge -n "__forge_at_depth3 config set" -a suggest -d "Set suggest model"
complete -c forge -n "__forge_at_depth3 config set" -a reasoning-effort -d "Set reasoning effort"

# config get targets
complete -c forge -n "__forge_at_depth3 config get" -a model -d "Get default model"
complete -c forge -n "__forge_at_depth3 config get" -a provider -d "Get default provider"
complete -c forge -n "__forge_at_depth3 config get" -a commit -d "Get commit model"
complete -c forge -n "__forge_at_depth3 config get" -a suggest -d "Get suggest model"
complete -c forge -n "__forge_at_depth3 config get" -a reasoning-effort -d "Get reasoning effort"

# ── conversation subcommands ──────────────────────────────────────────────────
complete -c forge -n "__forge_needs_subcommand conversation" -a list -d "List conversations"
complete -c forge -n "__forge_needs_subcommand conversation" -a new -d "Create new conversation"
complete -c forge -n "__forge_needs_subcommand conversation" -a dump -d "Export conversation"
complete -c forge -n "__forge_needs_subcommand conversation" -a compact -d "Compact conversation"
complete -c forge -n "__forge_needs_subcommand conversation" -a retry -d "Retry last turn"
complete -c forge -n "__forge_needs_subcommand conversation" -a resume -d "Resume conversation"
complete -c forge -n "__forge_needs_subcommand conversation" -a show -d "Show conversation"
complete -c forge -n "__forge_needs_subcommand conversation" -a info -d "Show conversation info"
complete -c forge -n "__forge_needs_subcommand conversation" -a stats -d "Show conversation stats"
complete -c forge -n "__forge_needs_subcommand conversation" -a clone -d "Clone conversation"
complete -c forge -n "__forge_needs_subcommand conversation" -a delete -d "Delete conversation"
complete -c forge -n "__forge_needs_subcommand conversation" -a rename -d "Rename conversation"

# ── list subcommands ──────────────────────────────────────────────────────────
complete -c forge -n "__forge_needs_subcommand list" -a agent -d "List agents"
complete -c forge -n "__forge_needs_subcommand list" -a provider -d "List providers"
complete -c forge -n "__forge_needs_subcommand list" -a model -d "List models"
complete -c forge -n "__forge_needs_subcommand list" -a command -d "List commands"
complete -c forge -n "__forge_needs_subcommand list" -a config -d "List config"
complete -c forge -n "__forge_needs_subcommand list" -a tool -d "List tools"
complete -c forge -n "__forge_needs_subcommand list" -a mcp -d "List MCP servers"
complete -c forge -n "__forge_needs_subcommand list" -a conversation -d "List conversations"
complete -c forge -n "__forge_needs_subcommand list" -a cmd -d "List custom commands"
complete -c forge -n "__forge_needs_subcommand list" -a skill -d "List skills"

# ── provider subcommands ──────────────────────────────────────────────────────
complete -c forge -n "__forge_needs_subcommand provider" -a login -d "Log in to provider"
complete -c forge -n "__forge_needs_subcommand provider" -a logout -d "Log out from provider"
complete -c forge -n "__forge_needs_subcommand provider" -a list -d "List providers"

# ── mcp subcommands ───────────────────────────────────────────────────────────
complete -c forge -n "__forge_needs_subcommand mcp" -a import -d "Import MCP server config"
complete -c forge -n "__forge_needs_subcommand mcp" -a list -d "List MCP servers"
complete -c forge -n "__forge_needs_subcommand mcp" -a remove -d "Remove MCP server"
complete -c forge -n "__forge_needs_subcommand mcp" -a show -d "Show MCP server details"
complete -c forge -n "__forge_needs_subcommand mcp" -a reload -d "Reload MCP servers"

# ── workspace subcommands ─────────────────────────────────────────────────────
complete -c forge -n "__forge_needs_subcommand workspace" -a sync -d "Sync workspace"
complete -c forge -n "__forge_needs_subcommand workspace" -a init -d "Initialize workspace"
complete -c forge -n "__forge_needs_subcommand workspace" -a status -d "Show workspace status"
complete -c forge -n "__forge_needs_subcommand workspace" -a info -d "Show workspace info"

# ── commit options ────────────────────────────────────────────────────────────
complete -c forge -n "__fish_seen_subcommand_from commit" -l max-diff -d "Maximum git diff size in bytes"
complete -c forge -n "__fish_seen_subcommand_from commit" -l preview -d "Preview commit message without committing"

# ── zsh subcommands ───────────────────────────────────────────────────────────
complete -c forge -n "__forge_needs_subcommand zsh" -a plugin -d "Generate shell plugin script"
complete -c forge -n "__forge_needs_subcommand zsh" -a theme -d "Generate shell theme"
complete -c forge -n "__forge_needs_subcommand zsh" -a doctor -d "Run diagnostics"
complete -c forge -n "__forge_needs_subcommand zsh" -a rprompt -d "Get rprompt information"
complete -c forge -n "__forge_needs_subcommand zsh" -a setup -d "Setup zsh integration"
complete -c forge -n "__forge_needs_subcommand zsh" -a keyboard -d "Show keyboard shortcuts"

# Common flags across subcommands
complete -c forge -n "__fish_seen_subcommand_from list agent provider mcp config conversation" -l porcelain -d "Machine-readable output"
complete -c forge -n "__fish_seen_subcommand_from list agent provider mcp config conversation" -s h -l help -d "Print help"
