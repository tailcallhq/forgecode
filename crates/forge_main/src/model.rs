use std::sync::{Arc, Mutex};

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use forge_api::{AgentInfo, Model, Template};
use forge_domain::UserCommand;
use strum::{EnumProperty, IntoEnumIterator};
use strum_macros::{EnumIter, EnumProperty};

use crate::info::Info;

/// Top-level Clap parser used to dispatch slash/colon commands.
///
/// The sentinel character (`/` or `:`) is stripped before passing tokens here,
/// so Clap only sees the subcommand name and its arguments.
#[derive(Debug, Parser)]
#[command(
    name = "forge_cmd",
    no_binary_name = true,
    disable_help_subcommand = true
)]
struct ClapCmd {
    #[command(subcommand)]
    sub: AppCommand,
}

/// Result of agent command registration
#[derive(Debug, Clone)]
pub struct AgentCommandRegistrationResult {
    pub registered_count: usize,
    pub skipped_conflicts: Vec<String>,
}

fn humanize_context_length(length: u64) -> String {
    if length >= 1_000_000 {
        format!("{:.1}M context", length as f64 / 1_000_000.0)
    } else if length >= 1_000 {
        format!("{:.1}K context", length as f64 / 1_000.0)
    } else {
        format!("{length} context")
    }
}

impl From<&[Model]> for Info {
    fn from(models: &[Model]) -> Self {
        let mut info = Info::new();

        for model in models.iter() {
            if let Some(context_length) = model.context_length {
                info = info.add_key_value(&model.id, humanize_context_length(context_length));
            } else {
                info = info.add_value(model.id.as_str());
            }
        }

        info
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeCommand {
    pub name: String,
    pub description: String,
    pub value: Option<String>,
}

#[derive(Debug)]
pub struct ForgeCommandManager {
    commands: Arc<Mutex<Vec<ForgeCommand>>>,
}

impl Default for ForgeCommandManager {
    fn default() -> Self {
        let commands = Self::default_commands();
        ForgeCommandManager { commands: Arc::new(Mutex::new(commands)) }
    }
}

impl ForgeCommandManager {
    /// Sanitizes agent ID to create a valid command name
    /// Replaces spaces and special characters with hyphens
    fn sanitize_agent_id(agent_id: &str) -> String {
        agent_id
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>()
            .join("-")
    }

    /// Checks if a command name conflicts with built-in commands
    fn is_reserved_command(name: &str) -> bool {
        matches!(
            name,
            "agent"
                | "forge"
                | "muse"
                | "sage"
                | "help"
                | "compact"
                | "new"
                | "info"
                | "usage"
                | "exit"
                | "update"
                | "dump"
                | "model"
                | "tools"
                | "provider"
                | "login"
                | "logout"
                | "retry"
                | "conversations"
                | "list"
                | "commit"
                | "rename"
                | "rn"
                | "config"
                | "env"
                | "config-model"
                | "cm"
                | "config-reload"
                | "cr"
                | "model-reset"
                | "mr"
                | "reasoning-effort"
                | "re"
                | "config-reasoning-effort"
                | "cre"
                | "config-commit-model"
                | "ccm"
                | "config-suggest-model"
                | "csm"
                | "config-edit"
                | "ce"
                | "skill"
                | "edit"
                | "ed"
                | "commit-preview"
                | "suggest"
                | "s"
                | "clone"
                | "conversation-rename"
                | "copy"
                | "workspace-sync"
                | "sync"
                | "workspace-status"
                | "sync-status"
                | "workspace-info"
                | "sync-info"
                | "workspace-init"
                | "sync-init"
        )
    }

    fn default_commands() -> Vec<ForgeCommand> {
        AppCommand::iter()
            .filter(|command| !command.is_internal())
            .map(|command| ForgeCommand {
                name: command.name().to_string(),
                description: command.usage().to_string(),
                value: None,
            })
            .collect::<Vec<_>>()
    }

    /// Registers workflow commands from the API.
    pub fn register_all(&self, commands: Vec<forge_domain::Command>) {
        let mut guard = self.commands.lock().unwrap();

        // Remove existing workflow commands (those with ⚙ prefix in description)
        guard.retain(|cmd| !cmd.description.starts_with("⚙ "));

        // Add new workflow commands
        let new_commands = commands.into_iter().map(|cmd| {
            let name = cmd.name.clone();
            let description = format!("⚙ {}", cmd.description);
            let value = cmd.prompt.clone();

            ForgeCommand { name, description, value }
        });

        guard.extend(new_commands);

        // Sort commands for consistent completion behavior
        guard.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Registers agent commands to the manager.
    /// Returns information about the registration process.
    pub fn register_agent_commands(
        &self,
        agents: Vec<AgentInfo>,
    ) -> AgentCommandRegistrationResult {
        let mut guard = self.commands.lock().unwrap();
        let mut result =
            AgentCommandRegistrationResult { registered_count: 0, skipped_conflicts: Vec::new() };

        // Remove existing agent commands (commands starting with "agent-")
        guard.retain(|cmd| !cmd.name.starts_with("agent-"));

        // Add new agent commands
        for agent in agents {
            let agent_id_str = agent.id.as_str();
            let sanitized_id = Self::sanitize_agent_id(agent_id_str);
            let command_name = format!("agent-{sanitized_id}");

            // Skip if it would conflict with reserved commands
            if Self::is_reserved_command(&command_name) {
                result.skipped_conflicts.push(command_name);
                continue;
            }

            let default_title = agent_id_str.to_string();
            let title = agent.title.as_ref().unwrap_or(&default_title);
            let description = format!("🤖 Switch to {title} agent");

            guard.push(ForgeCommand {
                name: command_name,
                description,
                value: Some(agent_id_str.to_string()),
            });

            result.registered_count += 1;
        }

        // Sort commands for consistent completion behavior
        guard.sort_by(|a, b| a.name.cmp(&b.name));

        result
    }

    /// Finds a command by name.
    fn find(&self, command: &str) -> Option<ForgeCommand> {
        self.commands
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.name == command)
            .cloned()
    }

    /// Lists all registered commands.
    pub fn list(&self) -> Vec<ForgeCommand> {
        self.commands.lock().unwrap().clone()
    }

    /// Extracts the command value from the input parts
    ///
    /// # Arguments
    /// * `command` - The command for which to extract the value
    /// * `parts` - The parts of the command input after the command name
    ///
    /// # Returns
    /// * `Option<String>` - The extracted value, if any
    fn extract_command_value(&self, command: &ForgeCommand, parts: &[&str]) -> Option<String> {
        // Unit tests implemented in the test module below

        // Try to get value provided in the command
        let value_provided = if !parts.is_empty() {
            Some(parts.join(" "))
        } else {
            None
        };

        // Try to get default value from command definition
        let value_default = self
            .commands
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.name == command.name)
            .and_then(|cmd| cmd.value.clone());

        // Use provided value if non-empty, otherwise use default
        match value_provided {
            Some(value) if !value.trim().is_empty() => Some(value),
            _ => value_default,
        }
    }

    pub fn parse(&self, input: &str) -> anyhow::Result<AppCommand> {
        // Shell commands (start with !) bypass Clap entirely.
        if input.trim().starts_with('!') {
            return Ok(AppCommand::Shell(
                input
                    .strip_prefix('!')
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            ));
        }

        let trimmed = input.trim();
        let mut tokens = trimmed.split_ascii_whitespace();
        let first = tokens.next().unwrap_or("");

        // Non-command input — pass straight through as a message.
        let is_command = first.starts_with('/') || first.starts_with(':');
        if !is_command {
            return Ok(AppCommand::Message(input.to_string()));
        }

        // Strip the sentinel character so Clap only sees the bare command name.
        let bare = first
            .strip_prefix('/')
            .or_else(|| first.strip_prefix(':'))
            .unwrap_or(first);
        let command_prefix = first
            .chars()
            .next()
            .filter(|c| *c == '/' || *c == ':')
            .unwrap_or(':');
        let rest: Vec<&str> = tokens.collect();

        // Build argv: [bare_command, arg1, arg2, …]
        let argv: Vec<&str> = std::iter::once(bare).chain(rest.iter().copied()).collect();
        let parameters: Vec<String> = rest.iter().map(|s| s.to_string()).collect();

        match ClapCmd::try_parse_from(&argv) {
            Ok(mut cmd) => {
                // Post-process variants that need Vec<String> → concrete type fixup
                match &mut cmd.sub {
                    AppCommand::Commit { args, max_diff_size } => {
                        *max_diff_size = args.iter().find_map(|p| p.parse::<usize>().ok());
                    }
                    AppCommand::Rename { name } => {
                        let n = name.join(" ");
                        let n = n.trim().to_string();
                        if n.is_empty() {
                            return Err(anyhow::anyhow!(
                                "Usage: :rename <name>. Please provide a name for the conversation."
                            ));
                        }
                    }
                    _ => {}
                }
                Ok(cmd.sub)
            }
            Err(clap_err) => {
                // Clap failed — check whether this is an agent command or a
                // registered custom workflow command before surfacing the error.
                let command_name = bare;

                // Give a domain-specific error for rename with no name argument.
                if (command_name == "rename" || command_name == "rn") && rest.is_empty() {
                    return Err(anyhow::anyhow!(
                        "Usage: :rename <name>. Please provide a name for the conversation."
                    ));
                }

                // Check if it's an agent command pattern (agent-*)
                if command_name.starts_with("agent-") {
                    if let Some(found_command) = self.find(command_name)
                        && let Some(agent_id) = &found_command.value
                    {
                        return Ok(AppCommand::AgentSwitch(agent_id.clone()));
                    }
                    return Err(anyhow::anyhow!(
                        "/{command_name} is not a valid agent command"
                    ));
                }

                // Handle custom workflow commands
                if let Some(command) = self.find(command_name) {
                    let rest_parts: Vec<&str> = rest.to_vec();
                    let template = Template::new(
                        self.extract_command_value(&command, &rest_parts)
                            .unwrap_or_default(),
                    );
                    return Ok(AppCommand::Custom(UserCommand::new(
                        command.name.clone(),
                        template,
                        parameters,
                    )));
                }

                // Surface user-friendly errors for unknown commands.
                if clap_err.kind() == ErrorKind::InvalidSubcommand {
                    return Err(anyhow::anyhow!(
                        "Unknown command '{command_prefix}{command_name}'. Run '{command_prefix}help' to list available commands."
                    ));
                }

                // Surface a clean error from Clap (strips ANSI + internal parser name).
                let rendered = clap_err.render().to_string();
                let cleaned = rendered.replace("forge_cmd", "forge");
                Err(anyhow::anyhow!("{}", cleaned.trim()))
            }
        }
    }
}

/// Represents user input types in the chat application.
///
/// This enum encapsulates all forms of input including:
/// - System commands (starting with '/')
/// - Regular chat messages
/// - File content
#[derive(Debug, Clone, PartialEq, Eq, EnumProperty, EnumIter, Subcommand)]
pub enum AppCommand {
    /// Display the effective resolved configuration.
    /// This can be triggered with the '/config' command (aliases: env, e).
    #[strum(props(usage = "Display effective resolved configuration"))]
    #[command(aliases = ["env", "e"])]
    Config,

    /// Set the global model via interactive selection.
    /// This can be triggered with the '/config-model' command (alias: cm).
    #[strum(props(usage = "Set the global model [alias: cm]"))]
    #[command(name = "config-model", alias = "cm")]
    ConfigModel,

    /// Reset session overrides to global config.
    /// This can be triggered with the '/config-reload' command (aliases: cr,
    /// model-reset, mr).
    #[strum(props(usage = "Reset session overrides to global config [alias: cr]"))]
    #[command(name = "config-reload", aliases = ["cr", "model-reset", "mr"])]
    ConfigReload,

    /// Set the reasoning effort level.
    /// This can be triggered with the '/reasoning-effort' command (alias: re).
    #[strum(props(usage = "Set reasoning effort for current session [alias: re]"))]
    #[command(name = "reasoning-effort", alias = "re")]
    ReasoningEffort,

    /// Set the reasoning effort level in global config.
    /// This can be triggered with the '/config-reasoning-effort' command
    /// (alias: cre).
    #[strum(props(usage = "Set reasoning effort in global config [alias: cre]"))]
    #[command(name = "config-reasoning-effort", alias = "cre")]
    ConfigReasoningEffort,

    /// Set the model used for commit message generation.
    /// This can be triggered with the '/config-commit-model' command (alias:
    /// ccm).
    #[strum(props(usage = "Set the model used for commit message generation [alias: ccm]"))]
    #[command(name = "config-commit-model", alias = "ccm")]
    ConfigCommitModel,

    /// Set the model used for command suggestion generation.
    /// This can be triggered with the '/config-suggest-model' command (alias:
    /// csm).
    #[strum(props(usage = "Set the model used for suggest generation [alias: csm]"))]
    #[command(name = "config-suggest-model", alias = "csm")]
    ConfigSuggestModel,

    /// Open the global config file in an editor.
    /// This can be triggered with the '/config-edit' command (alias: ce).
    #[strum(props(usage = "Open global config file in an editor [alias: ce]"))]
    #[command(name = "config-edit", alias = "ce")]
    ConfigEdit,

    /// List all available skills.
    /// This can be triggered with the '/skill' command.
    #[strum(props(usage = "List all available skills"))]
    Skill,

    /// Open an external editor to write a prompt.
    /// This can be triggered with the '/edit' command (alias: ed).
    #[strum(props(usage = "Open external editor to write a prompt [alias: ed]"))]
    #[command(alias = "ed")]
    Edit {
        /// Initial content for the editor (optional)
        #[arg(trailing_var_arg = true, num_args = 0..)]
        content: Vec<String>,
    },

    /// Preview the AI-generated commit message without committing.
    /// This can be triggered with the '/commit-preview' command.
    #[strum(props(usage = "Preview AI-generated commit message"))]
    #[command(name = "commit-preview")]
    CommitPreview,

    /// Generate a shell command from a natural language description.
    /// This can be triggered with the '/suggest' command (alias: s).
    #[strum(props(usage = "Generate shell command from natural language [alias: s]"))]
    #[command(alias = "s")]
    Suggest {
        /// Natural language description of the shell command
        #[arg(trailing_var_arg = true, num_args = 0.., allow_hyphen_values = true)]
        description: Vec<String>,
    },

    /// Clone the current or a selected conversation.
    /// This can be triggered with the '/clone' command.
    #[strum(props(usage = "Clone current or selected conversation"))]
    Clone {
        /// Conversation ID to clone (optional — prompts interactively if
        /// absent)
        id: Option<String>,
    },

    /// Rename any conversation interactively.
    /// This can be triggered with the '/conversation-rename' command.
    #[strum(props(usage = "Rename a conversation interactively"))]
    #[command(name = "conversation-rename")]
    ConversationRename {
        /// New name for the conversation (optional — prompts interactively if
        /// absent)
        #[arg(trailing_var_arg = true, num_args = 0..)]
        name: Vec<String>,
    },

    /// Copy the last AI response to the clipboard.
    /// This can be triggered with the '/copy' command.
    #[strum(props(usage = "Copy last AI response to clipboard"))]
    Copy,

    /// Sync the current workspace for semantic search.
    /// This can be triggered with the '/workspace-sync' command (alias: sync).
    #[strum(props(usage = "Sync current workspace for semantic search [alias: sync]"))]
    #[command(name = "workspace-sync", alias = "sync")]
    WorkspaceSync,

    /// Show sync status of all workspace files.
    /// This can be triggered with the '/workspace-status' command.
    #[strum(props(usage = "Show sync status of all workspace files"))]
    #[command(name = "workspace-status", alias = "sync-status")]
    WorkspaceStatus,

    /// Show workspace information with sync details.
    /// This can be triggered with the '/workspace-info' command.
    #[strum(props(usage = "Show workspace information with sync details"))]
    #[command(name = "workspace-info", alias = "sync-info")]
    WorkspaceInfo,

    /// Initialize a new workspace without syncing files.
    /// This can be triggered with the '/workspace-init' command.
    #[strum(props(usage = "Initialize a new workspace without syncing files"))]
    #[command(name = "workspace-init", alias = "sync-init")]
    WorkspaceInit,

    /// Compact the conversation context. This can be triggered with the
    /// '/compact' command.
    #[strum(props(usage = "Compact the conversation context"))]
    Compact,

    /// Start a new conversation while preserving history.
    /// This can be triggered with the '/new' command.
    #[strum(props(usage = "Start a new conversation"))]
    New,

    /// A regular text message from the user to be processed by the chat system.
    /// Any input that doesn't start with '/' is treated as a message.
    #[strum(props(usage = "Send a regular message"))]
    #[command(skip)]
    Message(String),

    /// Display system environment information.
    /// This can be triggered with the '/info' command.
    #[strum(props(usage = "Display system information"))]
    Info,

    /// Display usage information (tokens & requests).
    #[strum(props(usage = "Shows usage information (tokens & requests)"))]
    Usage,

    /// Exit the application without any further action.
    #[strum(props(usage = "Exit the application"))]
    Exit,

    /// Updates the forge version
    #[strum(props(usage = "Updates to the latest compatible version of forge"))]
    Update,

    /// Switch to "forge" agent.
    /// This can be triggered with the '/act' command (alias: forge).
    #[strum(props(usage = "Enable implementation mode with code changes"))]
    #[command(name = "act", alias = "forge")]
    Forge,

    /// Switch to "muse" agent.
    /// This can be triggered with the '/plan' command (alias: muse).
    #[strum(props(usage = "Enable planning mode without code changes"))]
    #[command(name = "plan", alias = "muse")]
    Muse,

    /// Switch to "sage" agent.
    /// This can be triggered with the '/sage' command.
    #[strum(props(
        usage = "Enable research mode for systematic codebase exploration and analysis"
    ))]
    Sage,

    /// Switch to "help" mode.
    /// This can be triggered with the '/help' command.
    #[strum(props(usage = "Enable help mode for tool questions"))]
    #[command(name = "help")]
    Help,

    /// Dumps the current conversation into a json file or html file
    #[strum(props(usage = "Save conversation as JSON or HTML (use /dump --html for HTML format)"))]
    Dump {
        /// Output as HTML instead of JSON
        #[arg(long)]
        html: bool,
    },

    /// Switch or select the active model
    /// This can be triggered with the '/model' command.
    #[strum(props(usage = "Switch to a different model"))]
    #[command(alias = "m")]
    Model,

    /// List all available tools with their descriptions and schema
    /// This can be triggered with the '/tools' command.
    #[strum(props(usage = "List all available tools with their descriptions and schema"))]
    #[command(alias = "t")]
    Tools,

    /// Handles custom command defined in workflow file.
    #[command(skip)]
    Custom(UserCommand),

    /// Executes a native shell command.
    /// This can be triggered with commands starting with '!' character.
    #[strum(props(usage = "Execute a native shell command"))]
    #[command(skip)]
    Shell(String),

    /// Allows user to switch the operating agent.
    #[strum(props(usage = "Switch to an agent interactively"))]
    #[command(alias = "a")]
    Agent,

    /// Allows you to configure provider
    #[strum(props(usage = "Allows you to configure provider"))]
    #[command(name = "provider", aliases = ["login", "provider-login"])]
    Login,

    /// Logs out from the configured provider
    #[strum(props(usage = "Logout from configured provider"))]
    Logout,

    /// Retry without modifying model context
    #[strum(props(usage = "Retry the last command"))]
    #[command(alias = "r")]
    Retry,

    /// List all conversations for the active workspace
    #[strum(props(usage = "List all conversations for the active workspace"))]
    #[command(name = "conversation", aliases = ["conversations", "c"])]
    Conversations {
        /// Conversation ID to switch to directly (optional — shows interactive
        /// picker if absent)
        id: Option<String>,
    },

    /// Delete a conversation permanently
    #[strum(props(usage = "Delete a conversation permanently"))]
    #[command(skip)]
    Delete,

    /// Rename the current conversation
    #[strum(props(usage = "Rename the current conversation. Usage: :rename <name>"))]
    #[command(alias = "rn")]
    Rename {
        /// New name for the conversation
        #[arg(trailing_var_arg = true, required = true)]
        name: Vec<String>,
    },

    /// Switch directly to a specific agent by ID
    #[strum(props(usage = "Switch directly to a specific agent"))]
    #[command(skip)]
    AgentSwitch(String),

    /// Generate and optionally commit changes with AI-generated message
    ///
    /// Examples:
    /// - `:commit` - Generate message and commit
    /// - `:commit 5000` - Commit with max diff of 5000 bytes
    #[strum(props(
        usage = "Generate AI commit message and commit changes. Format: :commit <max-diff|preview>"
    ))]
    Commit {
        /// Optional arguments (numeric value sets max diff size in bytes)
        #[arg(trailing_var_arg = true, num_args = 0..)]
        args: Vec<String>,
        /// Parsed max diff size (set by parse() from args)
        #[clap(skip)]
        max_diff_size: Option<usize>,
    },

    /// Index the current workspace for semantic code search
    #[strum(props(usage = "Index the current workspace for semantic search"))]
    Index,
}

impl AppCommand {
    pub fn name(&self) -> &str {
        match self {
            AppCommand::Compact => "compact",
            AppCommand::New => "new",
            AppCommand::Message(_) => "message",
            AppCommand::Update => "update",
            AppCommand::Info => "info",
            AppCommand::Usage => "usage",
            AppCommand::Exit => "exit",
            AppCommand::Forge => "forge",
            AppCommand::Muse => "muse",
            AppCommand::Sage => "sage",
            AppCommand::Help => "help",
            AppCommand::Commit { .. } => "commit",
            AppCommand::Dump { .. } => "dump",
            AppCommand::Model => "model",
            AppCommand::Tools => "tools",
            AppCommand::Custom(event) => &event.name,
            AppCommand::Shell(_) => "!shell",
            AppCommand::Agent => "agent",
            AppCommand::Login => "login",
            AppCommand::Logout => "logout",
            AppCommand::Retry => "retry",
            AppCommand::Conversations { .. } => "conversation",
            AppCommand::Delete => "delete",
            AppCommand::Rename { .. } => "rename",
            AppCommand::AgentSwitch(agent_id) => agent_id,
            AppCommand::Index => "index",
            AppCommand::Config => "config",
            AppCommand::ConfigModel => "config-model",
            AppCommand::ConfigReload => "config-reload",
            AppCommand::ReasoningEffort => "reasoning-effort",
            AppCommand::ConfigReasoningEffort => "config-reasoning-effort",
            AppCommand::ConfigCommitModel => "config-commit-model",
            AppCommand::ConfigSuggestModel => "config-suggest-model",
            AppCommand::ConfigEdit => "config-edit",
            AppCommand::Skill => "skill",
            AppCommand::Edit { .. } => "edit",
            AppCommand::CommitPreview => "commit-preview",
            AppCommand::Suggest { .. } => "suggest",
            AppCommand::Clone { .. } => "clone",
            AppCommand::ConversationRename { .. } => "conversation-rename",
            AppCommand::Copy => "copy",
            AppCommand::WorkspaceSync => "workspace-sync",
            AppCommand::WorkspaceStatus => "workspace-status",
            AppCommand::WorkspaceInfo => "workspace-info",
            AppCommand::WorkspaceInit => "workspace-init",
        }
    }

    /// Returns the usage description for the command.
    pub fn usage(&self) -> &str {
        self.get_str("usage").unwrap()
    }

    /// Returns true for internal/meta variants that should not appear in the
    /// public `forge list commands` output or the REPL help listing.
    pub fn is_internal(&self) -> bool {
        matches!(
            self,
            AppCommand::Message(_)
                | AppCommand::Custom(_)
                | AppCommand::Shell(_)
                | AppCommand::AgentSwitch(_)
                | AppCommand::Rename { .. }
        )
    }

    /// Returns true for variants that are pure agent-switch shorthands whose
    /// canonical name matches a built-in agent (forge, muse, sage).  These
    /// commands are already emitted as AGENT rows by the agent-info loop in
    /// `on_show_commands`, so they must be excluded from the COMMAND loop to
    /// avoid duplicate entries in `list commands --porcelain`.
    pub fn is_agent_switch(&self) -> bool {
        matches!(
            self,
            AppCommand::Forge | AppCommand::Muse | AppCommand::Sage
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Display;

    use colored::Colorize;
    use console::strip_ansi_codes;
    use forge_api::{
        AnyProvider, InputModality, Model, ModelId, ModelSource, ProviderId, ProviderResponse,
    };
    use forge_domain::Provider;
    use pretty_assertions::assert_eq;
    use url::Url;

    use super::*;
    use crate::display_constants::markers;

    /// Test-only wrapper for displaying models in selection menus
    #[derive(Clone)]
    struct CliModel(Model);

    impl Display for CliModel {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0.id)?;

            let mut info_parts = Vec::new();

            if let Some(limit) = self.0.context_length {
                if limit >= 1_000_000 {
                    info_parts.push(format!("{}M", limit / 1_000_000));
                } else if limit >= 1000 {
                    info_parts.push(format!("{}k", limit / 1000));
                } else {
                    info_parts.push(format!("{limit}"));
                }
            }

            if self.0.tools_supported == Some(true) {
                info_parts.push("🛠️".to_string());
            }

            if !info_parts.is_empty() {
                let info = format!("[ {} ]", info_parts.join(" "));
                write!(f, " {}", info.dimmed())?;
            }

            Ok(())
        }
    }

    /// Test-only wrapper for displaying providers in selection menus
    #[derive(Clone)]
    struct CliProvider(AnyProvider);

    impl Display for CliProvider {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let name_width = ProviderId::built_in_providers()
                .iter()
                .map(|id| id.to_string().len())
                .max()
                .unwrap_or(10);

            let name = self.0.id().to_string();

            match &self.0 {
                AnyProvider::Url(provider) => {
                    write!(f, "{} {:<width$}", "✓".green(), name, width = name_width)?;
                    if let Some(domain) = provider.url.domain() {
                        write!(f, " [{domain}]")?;
                    } else {
                        write!(f, " {}", markers::EMPTY)?;
                    }
                }
                AnyProvider::Template(_) => {
                    write!(f, "  {name:<name_width$} {}", markers::EMPTY)?;
                }
            }
            Ok(())
        }
    }

    #[test]
    fn test_extract_command_value_with_provided_value() {
        // Setup
        let cmd_manager = ForgeCommandManager::default();
        let command = ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: None,
        };
        let parts = vec!["arg1", "arg2"];

        // Execute
        let result = cmd_manager.extract_command_value(&command, &parts);

        // Verify
        assert_eq!(result, Some(String::from("arg1 arg2")));
    }

    #[test]
    fn test_extract_command_value_with_empty_parts_default_value() {
        // Setup
        let cmd_manager = ForgeCommandManager {
            commands: Arc::new(Mutex::new(vec![ForgeCommand {
                name: String::from("/test"),
                description: String::from("Test command"),
                value: Some(String::from("default_value")),
            }])),
        };
        let command = ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: None,
        };
        let parts: Vec<&str> = vec![];

        // Execute
        let result = cmd_manager.extract_command_value(&command, &parts);

        // Verify
        assert_eq!(result, Some(String::from("default_value")));
    }

    #[test]
    fn test_extract_command_value_with_empty_string_parts() {
        // Setup
        let cmd_manager = ForgeCommandManager {
            commands: Arc::new(Mutex::new(vec![ForgeCommand {
                name: String::from("/test"),
                description: String::from("Test command"),
                value: Some(String::from("default_value")),
            }])),
        };
        let command = ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: None,
        };
        let parts = vec![""];

        // Execute
        let result = cmd_manager.extract_command_value(&command, &parts);

        // Verify - should use default as the provided value is empty
        assert_eq!(result, Some(String::from("default_value")));
    }

    #[test]
    fn test_extract_command_value_with_whitespace_parts() {
        // Setup
        let cmd_manager = ForgeCommandManager {
            commands: Arc::new(Mutex::new(vec![ForgeCommand {
                name: String::from("/test"),
                description: String::from("Test command"),
                value: Some(String::from("default_value")),
            }])),
        };
        let command = ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: None,
        };
        let parts = vec!["  "];

        // Execute
        let result = cmd_manager.extract_command_value(&command, &parts);

        // Verify - should use default as the provided value is just whitespace
        assert_eq!(result, Some(String::from("default_value")));
    }

    #[test]
    fn test_extract_command_value_no_default_no_provided() {
        // Setup
        let cmd_manager = ForgeCommandManager {
            commands: Arc::new(Mutex::new(vec![ForgeCommand {
                name: String::from("/test"),
                description: String::from("Test command"),
                value: None,
            }])),
        };
        let command = ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: None,
        };
        let parts: Vec<&str> = vec![];

        // Execute
        let result = cmd_manager.extract_command_value(&command, &parts);

        // Verify - should be None as there's no default and no provided value
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_command_value_provided_overrides_default() {
        // Setup
        let cmd_manager = ForgeCommandManager {
            commands: Arc::new(Mutex::new(vec![ForgeCommand {
                name: String::from("/test"),
                description: String::from("Test command"),
                value: Some(String::from("default_value")),
            }])),
        };
        let command = ForgeCommand {
            name: String::from("/test"),
            description: String::from("Test command"),
            value: None,
        };
        let parts = vec!["provided_value"];

        // Execute
        let result = cmd_manager.extract_command_value(&command, &parts);

        // Verify - provided value should override default
        assert_eq!(result, Some(String::from("provided_value")));
    }
    #[test]
    fn test_parse_shell_command() {
        // Setup
        let cmd_manager = ForgeCommandManager::default();

        // Execute
        let result = cmd_manager.parse("!ls -la").unwrap();

        // Verify
        match result {
            AppCommand::Shell(cmd) => assert_eq!(cmd, "ls -la"),
            _ => panic!("Expected Shell command, got {result:?}"),
        }
    }

    #[test]
    fn test_parse_shell_command_empty() {
        // Setup
        let cmd_manager = ForgeCommandManager::default();

        // Execute
        let result = cmd_manager.parse("!").unwrap();

        // Verify
        match result {
            AppCommand::Shell(cmd) => assert_eq!(cmd, ""),
            _ => panic!("Expected Shell command, got {result:?}"),
        }
    }

    #[test]
    fn test_parse_shell_command_with_whitespace() {
        // Setup
        let cmd_manager = ForgeCommandManager::default();

        // Execute
        let result = cmd_manager.parse("!   echo 'test'   ").unwrap();

        // Verify
        match result {
            AppCommand::Shell(cmd) => assert_eq!(cmd, "echo 'test'"),
            _ => panic!("Expected Shell command, got {result:?}"),
        }
    }

    #[test]
    fn test_shell_command_not_in_default_commands() {
        // Setup
        let manager = ForgeCommandManager::default();
        let commands = manager.list();

        // The shell command should not be included
        let contains_shell = commands.iter().any(|cmd| cmd.name == "!shell");
        assert!(
            !contains_shell,
            "Shell command should not be in default commands"
        );
    }
    #[test]
    fn test_parse_list_command() {
        // Setup
        let cmd_manager = ForgeCommandManager::default();

        // Execute
        let result = cmd_manager.parse("/conversation").unwrap();

        // Verify
        match result {
            AppCommand::Conversations { .. } => {
                // Command parsed correctly
            }
            _ => panic!("Expected List command, got {result:?}"),
        }
    }

    #[test]
    fn test_parse_conversation_with_id() {
        // Setup
        let cmd_manager = ForgeCommandManager::default();

        // Execute
        let actual = cmd_manager
            .parse("/conversation 550e8400-e29b-41d4-a716-446655440000")
            .unwrap();

        // Verify
        let expected = AppCommand::Conversations {
            id: Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_list_command_in_default_commands() {
        // Setup
        let manager = ForgeCommandManager::default();
        let commands = manager.list();

        // The list command should be included
        let contains_list = commands.iter().any(|cmd| cmd.name == "conversation");
        assert!(
            contains_list,
            "Conversations command should be in default commands"
        );
    }

    #[test]
    fn test_sanitize_agent_id_basic() {
        // Test basic sanitization
        let fixture = "test-agent";
        let actual = ForgeCommandManager::sanitize_agent_id(fixture);
        let expected = "test-agent";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_agent_id_with_spaces() {
        // Test space replacement
        let fixture = "test agent name";
        let actual = ForgeCommandManager::sanitize_agent_id(fixture);
        let expected = "test-agent-name";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_agent_id_with_special_chars() {
        // Test special character replacement
        let fixture = "test@agent#name!";
        let actual = ForgeCommandManager::sanitize_agent_id(fixture);
        let expected = "test-agent-name";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_agent_id_uppercase() {
        // Test uppercase conversion
        let fixture = "TestAgent";
        let actual = ForgeCommandManager::sanitize_agent_id(fixture);
        let expected = "testagent";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_is_reserved_command() {
        // Test reserved commands
        assert!(ForgeCommandManager::is_reserved_command("agent"));
        assert!(ForgeCommandManager::is_reserved_command("forge"));
        assert!(ForgeCommandManager::is_reserved_command("muse"));
        assert!(!ForgeCommandManager::is_reserved_command("agent-custom"));
        assert!(!ForgeCommandManager::is_reserved_command("custom"));
    }

    #[test]
    fn test_register_agent_commands() {
        // Setup
        let fixture = ForgeCommandManager::default();
        let agents = vec![
            forge_domain::AgentInfo::default()
                .id("test-agent")
                .title("Test Agent".to_string()),
            forge_domain::AgentInfo::default()
                .id("another")
                .title("Another Agent".to_string()),
        ];

        // Execute
        let result = fixture.register_agent_commands(agents);

        // Verify result
        assert_eq!(result.registered_count, 2);
        assert_eq!(result.skipped_conflicts.len(), 0);

        // Verify
        let commands = fixture.list();
        let agent_commands: Vec<_> = commands
            .iter()
            .filter(|cmd| cmd.name.starts_with("agent-"))
            .collect();

        assert_eq!(agent_commands.len(), 2);
        assert!(
            agent_commands
                .iter()
                .any(|cmd| cmd.name == "agent-test-agent")
        );
        assert!(agent_commands.iter().any(|cmd| cmd.name == "agent-another"));
    }

    #[test]
    fn test_parse_agent_switch_command() {
        // Setup
        let fixture = ForgeCommandManager::default();
        let agents = vec![
            forge_domain::AgentInfo::default()
                .id("test-agent")
                .title("Test Agent".to_string()),
        ];
        let _result = fixture.register_agent_commands(agents);

        // Execute
        let actual = fixture.parse("/agent-test-agent").unwrap();

        // Verify
        match actual {
            AppCommand::AgentSwitch(agent_id) => assert_eq!(agent_id, "test-agent"),
            _ => panic!("Expected AgentSwitch command, got {actual:?}"),
        }
    }

    fn create_model_fixture(
        id: &str,
        context_length: Option<u64>,
        tools_supported: Option<bool>,
    ) -> Model {
        Model {
            id: ModelId::new(id),
            name: None,
            description: None,
            context_length,
            tools_supported,
            supports_parallel_tool_calls: None,
            supports_reasoning: None,
            input_modalities: vec![InputModality::Text],
        }
    }

    #[test]
    fn test_cli_model_display_with_context_and_tools() {
        let fixture = create_model_fixture("gpt-4", Some(128000), Some(true));
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "gpt-4 [ 128k 🛠️ ]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_model_display_with_large_context() {
        let fixture = create_model_fixture("claude-3", Some(2000000), Some(true));
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "claude-3 [ 2M 🛠️ ]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_model_display_with_small_context() {
        let fixture = create_model_fixture("small-model", Some(512), Some(false));
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "small-model [ 512 ]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_model_display_with_context_only() {
        let fixture = create_model_fixture("text-model", Some(4096), Some(false));
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "text-model [ 4k ]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_model_display_with_tools_only() {
        let fixture = create_model_fixture("tool-model", None, Some(true));
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "tool-model [ 🛠️ ]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_model_display_empty_context_and_no_tools() {
        let fixture = create_model_fixture("basic-model", None, Some(false));
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "basic-model";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_model_display_empty_context_and_none_tools() {
        let fixture = create_model_fixture("unknown-model", None, None);
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "unknown-model";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_model_display_exact_thousands() {
        let fixture = create_model_fixture("exact-k", Some(8000), Some(true));
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "exact-k [ 8k 🛠️ ]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_model_display_exact_millions() {
        let fixture = create_model_fixture("exact-m", Some(1000000), Some(true));
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "exact-m [ 1M 🛠️ ]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_model_display_edge_case_999() {
        let fixture = create_model_fixture("edge-999", Some(999), None);
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "edge-999 [ 999 ]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_model_display_edge_case_1001() {
        let fixture = create_model_fixture("edge-1001", Some(1001), None);
        let formatted = format!("{}", CliModel(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "edge-1001 [ 1k ]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_provider_display_minimal() {
        let fixture = AnyProvider::Url(Provider {
            id: ProviderId::OPENAI,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: None,
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.openai.com/v1/models").unwrap(),
            )),
        });
        let formatted = format!("{}", CliProvider(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "✓ OpenAI                    [api.openai.com]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_provider_display_with_subdomain() {
        let fixture = AnyProvider::Url(Provider {
            id: ProviderId::OPEN_ROUTER,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://openrouter.ai/api/v1/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: None,
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://openrouter.ai/api/v1/models").unwrap(),
            )),
        });
        let formatted = format!("{}", CliProvider(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "✓ OpenRouter                [openrouter.ai]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_provider_display_no_domain() {
        let fixture = AnyProvider::Url(Provider {
            id: ProviderId::FORGE,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("http://localhost:8080/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: None,
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("http://localhost:8080/models").unwrap(),
            )),
        });
        let formatted = format!("{}", CliProvider(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = "✓ Forge                     [localhost]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_provider_display_template() {
        let fixture = AnyProvider::Template(Provider {
            id: ProviderId::ANTHROPIC,
            provider_type: Default::default(),
            response: Some(ProviderResponse::Anthropic),
            url: Template::new("https://api.anthropic.com/v1/messages"),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: None,
            custom_headers: None,
            models: Some(ModelSource::Url(Template::new(
                "https://api.anthropic.com/v1/models",
            ))),
        });
        let formatted = format!("{}", CliProvider(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = format!("  Anthropic                 {}", markers::EMPTY);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cli_provider_display_ip_address() {
        let fixture = AnyProvider::Url(Provider {
            id: ProviderId::FORGE,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("http://192.168.1.1:8080/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: None,
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("http://192.168.1.1:8080/models").unwrap(),
            )),
        });
        let formatted = format!("{}", CliProvider(fixture));
        let actual = strip_ansi_codes(&formatted);
        let expected = format!("✓ Forge                     {}", markers::EMPTY);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_commit_command() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/commit").unwrap();
        match actual {
            AppCommand::Commit { max_diff_size, .. } => {
                assert_eq!(max_diff_size, None);
            }
            _ => panic!("Expected Commit command, got {actual:?}"),
        }
    }

    #[test]
    fn test_parse_commit_command_with_preview() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/commit preview").unwrap();
        match actual {
            AppCommand::Commit { max_diff_size, .. } => {
                assert_eq!(max_diff_size, None);
            }
            _ => panic!("Expected Commit command with preview, got {actual:?}"),
        }
    }

    #[test]
    fn test_parse_commit_command_with_max_diff() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/commit 5000").unwrap();
        match actual {
            AppCommand::Commit { max_diff_size, .. } => {
                assert_eq!(max_diff_size, Some(5000));
            }
            _ => panic!("Expected Commit command with max_diff_size, got {actual:?}"),
        }
    }

    #[test]
    fn test_parse_commit_command_with_all_flags() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/commit preview 10000").unwrap();
        match actual {
            AppCommand::Commit { max_diff_size, .. } => {
                assert_eq!(max_diff_size, Some(10000));
            }
            _ => panic!("Expected Commit command with all flags, got {actual:?}"),
        }
    }

    #[test]
    fn test_commit_command_in_default_commands() {
        let manager = ForgeCommandManager::default();
        let commands = manager.list();
        let contains_commit = commands.iter().any(|cmd| cmd.name == "commit");
        assert!(
            contains_commit,
            "Commit command should be in default commands"
        );
    }

    #[test]
    fn test_parse_invalid_agent_command() {
        // Setup
        let fixture = ForgeCommandManager::default();

        // Execute
        let result = fixture.parse("/agent-nonexistent");

        // Verify
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not a valid agent command")
        );
    }

    #[test]
    fn test_parse_invalid_command_with_colon_returns_helpful_error() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse(":celar").unwrap_err().to_string();
        let expected =
            "Unknown command ':celar'. Run ':help' to list available commands.".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_invalid_command_with_slash_returns_helpful_error() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/celar").unwrap_err().to_string();
        let expected =
            "Unknown command '/celar'. Run '/help' to list available commands.".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_tool_command() {
        // Setup
        let fixture = ForgeCommandManager::default();

        // Execute
        let result = fixture.parse("/tools").unwrap();

        // Verify
        match result {
            AppCommand::Tools => {
                // Command parsed correctly
            }
            _ => panic!("Expected Tool command, got {result:?}"),
        }
    }

    #[test]
    fn test_parse_dump_command_json() {
        // Setup
        let fixture = ForgeCommandManager::default();

        // Execute
        let actual = fixture.parse("/dump").unwrap();

        // Verify
        let expected = AppCommand::Dump { html: false };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_dump_command_html_with_flag() {
        // Setup
        let fixture = ForgeCommandManager::default();

        // Execute
        let actual = fixture.parse("/dump --html").unwrap();

        // Verify
        let expected = AppCommand::Dump { html: true };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_rename_command() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/rename my-session").unwrap();
        assert_eq!(
            actual,
            AppCommand::Rename { name: vec!["my-session".to_string()] }
        );
    }

    #[test]
    fn test_parse_rename_command_multi_word() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/rename auth refactor work").unwrap();
        assert_eq!(
            actual,
            AppCommand::Rename {
                name: vec![
                    "auth".to_string(),
                    "refactor".to_string(),
                    "work".to_string()
                ]
            }
        );
    }

    #[test]
    fn test_parse_rename_command_no_name() {
        let fixture = ForgeCommandManager::default();
        let result = fixture.parse("/rename");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("provide a name"));
    }

    #[test]
    fn test_parse_rename_alias() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/rn my-session").unwrap();
        assert_eq!(
            actual,
            AppCommand::Rename { name: vec!["my-session".to_string()] }
        );
    }

    #[test]
    fn test_parse_rename_trims_whitespace() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/rename   my title   ").unwrap();
        assert_eq!(
            actual,
            AppCommand::Rename { name: vec!["my".to_string(), "title".to_string()] }
        );
    }

    #[test]
    fn test_rename_is_reserved_command() {
        assert!(ForgeCommandManager::is_reserved_command("rename"));
        assert!(ForgeCommandManager::is_reserved_command("rn"));
    }

    #[test]
    fn test_rename_command_name() {
        let cmd = AppCommand::Rename { name: vec!["test".to_string()] };
        assert_eq!(cmd.name(), "rename");
    }

    #[test]
    fn test_parse_suggest_with_dash_prefixed_tokens() {
        // Setup
        let cmd_manager = ForgeCommandManager::default();

        // Execute
        let result = cmd_manager.parse(":suggest --- date").unwrap();

        // Verify
        assert_eq!(
            result,
            AppCommand::Suggest { description: vec!["---".to_string(), "date".to_string()] }
        );
    }

    #[test]
    fn test_parse_suggest_with_double_dash_flags() {
        // Setup
        let cmd_manager = ForgeCommandManager::default();

        // Execute
        let result = cmd_manager.parse(":suggest --date tomorrow").unwrap();

        // Verify
        assert_eq!(
            result,
            AppCommand::Suggest {
                description: vec!["--date".to_string(), "tomorrow".to_string()]
            }
        );
    }

    #[test]
    fn test_parse_suggest_with_single_dash() {
        // Setup
        let cmd_manager = ForgeCommandManager::default();

        // Execute
        let result = cmd_manager.parse(":suggest -v file.txt").unwrap();

        // Verify
        assert_eq!(
            result,
            AppCommand::Suggest { description: vec!["-v".to_string(), "file.txt".to_string()] }
        );
    }
}
