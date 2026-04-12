//! NOTE: Always use singular names for commands and subcommands.
//! For example: `forge provider login` instead of `forge providers login`.
//!
//! NOTE: With every change to this CLI structure, verify that the ZSH plugin
//! remains compatible. The plugin at `shell-plugin/forge.plugin.zsh` implements
//! shell completion and command shortcuts that depend on the CLI structure.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use forge_domain::{AgentId, ConversationId, Effort, ModelId, ProviderId};

#[derive(Parser)]
#[command(version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    /// Direct prompt to process without entering interactive mode.
    ///
    /// When provided, executes a single command and exits instead of starting
    /// an interactive session. Content can also be piped: `cat prompt.txt |
    /// forge`.
    #[arg(long, short = 'p', allow_hyphen_values = true)]
    pub prompt: Option<String>,

    /// Piped input from stdin (populated internally)
    ///
    /// This field is automatically populated when content is piped to forge
    /// via stdin. It's kept separate from the prompt to allow proper handling
    /// as a droppable message.
    #[arg(skip)]
    pub piped_input: Option<String>,

    /// Path to a JSON file containing the conversation to execute.
    #[arg(long)]
    pub conversation: Option<PathBuf>,

    /// Conversation ID to use for this session.
    ///
    /// When provided, resumes or continues an existing conversation instead of
    /// generating a new conversation ID.
    #[arg(long, alias = "cid")]
    pub conversation_id: Option<ConversationId>,

    /// Working directory to use before starting the session.
    ///
    /// When provided, changes to this directory before starting forge.
    #[arg(long, short = 'C')]
    pub directory: Option<PathBuf>,

    /// Name for an isolated git worktree to create for experimentation.
    #[arg(long)]
    pub sandbox: Option<String>,

    /// Enable verbose logging output.
    #[arg(long, default_value_t = false)]
    pub verbose: bool,

    /// Agent ID to use for this session.
    #[arg(long, alias = "aid")]
    pub agent: Option<AgentId>,

    /// Top-level subcommands.
    #[command(subcommand)]
    pub subcommands: Option<TopLevelCommand>,

    /// Event to dispatch to the workflow in JSON format.
    #[arg(long, short = 'e')]
    pub event: Option<String>,
}

impl Cli {
    /// Determines whether the CLI should start in interactive mode.
    ///
    /// Returns true when no prompt, piped input, or subcommand is provided,
    /// indicating the user wants to enter interactive mode.
    pub fn is_interactive(&self) -> bool {
        self.prompt.is_none() && self.piped_input.is_none() && self.subcommands.is_none()
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum TopLevelCommand {
    /// Manage agents.
    Agent(AgentCommandGroup),

    /// Generate shell extension scripts.
    #[command(subcommand, alias = "extension")]
    Zsh(ZshCommandGroup),

    /// List agents, models, providers, tools, or MCP servers.
    List(ListCommandGroup),

    /// Display the banner with version information.
    Banner,

    /// Show configuration, active model, and environment status.
    Info {
        /// Conversation ID for session-specific information.
        #[arg(long, alias = "cid")]
        conversation_id: Option<ConversationId>,

        /// Output in machine-readable format.
        #[arg(long)]
        porcelain: bool,
    },

    /// Get, set, or list configuration values.
    Config(ConfigCommandGroup),

    /// Manage conversation history and state.
    #[command(alias = "session")]
    Conversation(ConversationCommandGroup),

    /// Generate and optionally commit changes with AI-generated message
    Commit(CommitCommandGroup),

    /// Manage Model Context Protocol servers.
    Mcp(McpCommandGroup),

    /// Suggest shell commands from natural language.
    Suggest {
        /// Natural language description of the desired command.
        prompt: String,
    },

    /// Manage API provider authentication.
    Provider(ProviderCommandGroup),

    /// Run or list custom commands.
    #[command(aliases = ["command", "commands"])]
    Cmd(CmdCommandGroup),

    /// Manage workspaces for semantic search.
    Workspace(WorkspaceCommandGroup),

    /// Process JSONL data through LLM with schema-constrained tools.
    Data(DataCommandGroup),

    /// VS Code integration commands.
    #[command(subcommand)]
    Vscode(VscodeCommand),

    /// Paste image from clipboard and return the @[path] string.
    PasteImage {
        /// Optional path to save the image to.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },

    /// Update forge to the latest version.
    Update(UpdateArgs),

    /// Setup zsh integration by updating .zshrc with plugin and theme (alias
    /// for `zsh setup`).
    Setup,

    /// Run diagnostics on shell environment (alias for `zsh doctor`).
    Doctor,
}

/// Command group for custom command management.
#[derive(Parser, Debug, Clone)]
pub struct CmdCommandGroup {
    #[command(subcommand)]
    pub command: CmdCommand,

    /// Conversation ID to execute the command within.
    #[arg(long, alias = "cid", global = true)]
    pub conversation_id: Option<ConversationId>,

    /// Output in machine-readable format.
    #[arg(long, global = true)]
    pub porcelain: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CmdCommand {
    /// List all available custom commands.
    List {
        /// Shows only custom commands
        #[arg(long)]
        custom: bool,
    },

    /// Execute a custom command.
    Execute {
        /// Name of the custom command to execute, followed by any arguments.
        commands: Vec<String>,
    },
}

/// Command group for agent management.
#[derive(Parser, Debug, Clone)]
pub struct AgentCommandGroup {
    #[command(subcommand)]
    pub command: AgentCommand,

    /// Output in machine-readable format.
    #[arg(long, global = true)]
    pub porcelain: bool,
}

/// Agent management commands.
#[derive(Subcommand, Debug, Clone)]
pub enum AgentCommand {
    /// List available agents.
    #[command(alias = "ls")]
    List,
}

/// Command group for workspace management.
#[derive(Parser, Debug, Clone)]
pub struct WorkspaceCommandGroup {
    #[command(subcommand)]
    pub command: WorkspaceCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum WorkspaceCommand {
    /// Synchronize a directory for semantic search.
    Sync {
        /// Path to the directory to sync
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Automatically initialize the workspace before syncing if it has not
        /// been initialized yet.
        #[arg(long)]
        init: bool,
    },
    /// List all workspaces.
    List {
        /// Output in machine-readable format
        #[arg(short, long)]
        porcelain: bool,
    },

    /// Query the workspace.
    Query {
        /// Search query.
        query: String,

        /// Path to the directory to index (used when no subcommand is
        /// provided).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum number of results to return.
        #[arg(short, long, default_value = "10")]
        limit: usize,

        /// Number of highest probability tokens to consider (1-1000).
        #[arg(long)]
        top_k: Option<u32>,

        /// Describe your intent or goal to filter results for relevance.
        #[arg(long, short = 'r')]
        use_case: String,

        /// Filter results to files starting with this prefix.
        #[arg(long)]
        starts_with: Option<String>,

        /// Filter results to files ending with this suffix.
        #[arg(long)]
        ends_with: Option<String>,
    },

    /// Show workspace information for an indexed directory.
    Info {
        /// Path to the directory to get information for
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Delete one or more workspaces.
    Delete {
        /// Workspace IDs to delete
        workspace_ids: Vec<String>,
    },

    /// Show sync status of all files in the workspace.
    Status {
        /// Path to the directory to check status for
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output in machine-readable format
        #[arg(short, long)]
        porcelain: bool,
    },

    /// Initialize an empty workspace for the provided directory
    Init {
        /// Path to the directory to initialize as a workspace
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Automatically confirm initialization without prompting
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

/// Command group for listing resources.
#[derive(Parser, Debug, Clone)]
pub struct ListCommandGroup {
    #[command(subcommand)]
    pub command: ListCommand,

    /// Output in machine-readable format.
    #[arg(long, global = true)]
    pub porcelain: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ListCommand {
    /// List available agents.
    #[command(alias = "agents")]
    Agent {
        /// Shows only custom agents
        #[arg(long)]
        custom: bool,
    },

    /// List available API providers.
    #[command(alias = "providers")]
    Provider {
        /// Filter providers by type (e.g., llm, context_engine). Can be
        /// specified multiple times.
        #[arg(long = "type", short = 't')]
        types: Vec<forge_domain::ProviderType>,
    },

    /// List available models.
    #[command(alias = "models")]
    Model,

    /// List available commands.
    #[command(hide = true, alias = "commands")]
    Command {
        /// Shows only custom commands
        #[arg(long)]
        custom: bool,
    },

    /// List configuration values.
    #[command(alias = "configs")]
    Config,

    /// List tools for a specific agent.
    #[command(alias = "tools")]
    Tool {
        /// Agent ID to list tools for.
        agent: AgentId,
    },

    /// List MCP servers.
    #[command(alias = "mcps")]
    Mcp,

    /// List conversation history.
    #[command(alias = "session")]
    Conversation,

    /// List custom commands.
    #[command(alias = "cmds")]
    Cmd,

    /// List available skills.
    #[command(alias = "skills")]
    Skill {
        /// Shows only custom skills
        #[arg(long)]
        custom: bool,
    },
}

/// Shell extension commands.
#[derive(Subcommand, Debug, Clone)]
pub enum ZshCommandGroup {
    /// Generate shell plugin script
    Plugin,
    /// Generate shell theme
    Theme,
    /// Run diagnostics on shell environment
    Doctor,

    /// Get rprompt information (model and conversation stats) for shell
    /// integration.
    Rprompt,

    /// Setup zsh integration by updating .zshrc with plugin and theme
    Setup,

    /// Show keyboard shortcuts for ZSH line editor
    Keyboard,

    /// Format buffer text by wrapping file paths in @[...] syntax.
    ///
    /// Used by the zsh plugin to delegate path detection and wrapping to
    /// Rust where the logic is well-tested across all terminal environments.
    Format {
        /// The text buffer to format.
        #[arg(long)]
        buffer: String,
    },
}

/// Command group for MCP server management.
#[derive(Parser, Debug, Clone)]
pub struct McpCommandGroup {
    #[command(subcommand)]
    pub command: McpCommand,

    /// Output in machine-readable format.
    #[arg(long, global = true)]
    pub porcelain: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum McpCommand {
    /// Import server configuration from JSON.
    Import(McpImportArgs),

    /// List configured servers.
    List,

    /// Remove a configured server.
    Remove(McpRemoveArgs),

    /// Show server configuration details.
    Show(McpShowArgs),

    /// Reload servers and rebuild caches.
    Reload,

    /// Authenticate with an OAuth-enabled MCP server.
    Login(McpAuthArgs),

    /// Remove stored OAuth credentials for an MCP server.
    Logout(McpLogoutArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct McpImportArgs {
    /// JSON configuration to import.
    #[arg()]
    pub json: String,

    /// Configuration scope.
    #[arg(short = 's', long = "scope", default_value = "local")]
    pub scope: Scope,
}

#[derive(Parser, Debug, Clone)]
pub struct McpRemoveArgs {
    /// Configuration scope.
    #[arg(short = 's', long = "scope", default_value = "local")]
    pub scope: Scope,

    /// Name of the server to remove.
    pub name: String,
}

#[derive(Parser, Debug, Clone)]
pub struct McpShowArgs {
    /// Name of the server to show details for.
    pub name: String,
}

#[derive(Parser, Debug, Clone)]
pub struct McpAuthArgs {
    /// Name of the MCP server to authenticate with.
    pub name: String,
}

#[derive(Parser, Debug, Clone)]
pub struct McpLogoutArgs {
    /// Name of the MCP server to remove credentials for, or "all" to
    /// remove all MCP OAuth credentials.
    pub name: String,
}

/// Configuration scope for settings.
#[derive(Copy, Clone, Debug, ValueEnum, Default)]
pub enum Scope {
    /// Local configuration (project-specific).
    #[default]
    Local,
    /// User configuration (global to the user).
    User,
}

impl From<Scope> for forge_domain::Scope {
    fn from(value: Scope) -> Self {
        match value {
            Scope::Local => forge_domain::Scope::Local,
            Scope::User => forge_domain::Scope::User,
        }
    }
}

/// Transport protocol for communication.
#[derive(Copy, Clone, Debug, ValueEnum)]
#[clap(rename_all = "lower")]
pub enum Transport {
    /// Standard input/output communication.
    Stdio,
    /// Server-sent events communication.
    Sse,
}

/// Command group for configuration management.
#[derive(Parser, Debug, Clone)]
pub struct ConfigCommandGroup {
    #[command(subcommand)]
    pub command: ConfigCommand,

    /// Output in machine-readable format.
    #[arg(long, global = true)]
    pub porcelain: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCommand {
    /// Set a configuration value.
    Set(ConfigSetArgs),

    /// Get a configuration value.
    Get(ConfigGetArgs),

    /// List configuration values.
    List,

    /// Print the path to the global config file.
    Path,

    /// Migrate the legacy ~/forge directory to ~/.forge.
    Migrate,
}

/// Arguments for `forge config set`.
#[derive(Parser, Debug, Clone)]
pub struct ConfigSetArgs {
    #[command(subcommand)]
    pub field: ConfigSetField,
}

/// Arguments for `forge config get`.
#[derive(Parser, Debug, Clone)]
pub struct ConfigGetArgs {
    #[command(subcommand)]
    pub field: ConfigGetField,
}

/// Type-safe subcommands for `forge config set`.
#[derive(Subcommand, Debug, Clone)]
pub enum ConfigSetField {
    /// Set the active model and provider atomically.
    Model {
        /// Provider ID to set as default.
        provider: ProviderId,
        /// Model ID to set as default.
        model: ModelId,
    },
    /// Set the provider and model for commit message generation.
    Commit {
        /// Provider ID to use for commit message generation.
        provider: ProviderId,
        /// Model ID to use for commit message generation.
        model: ModelId,
    },
    /// Set the provider and model for command suggestion generation.
    Suggest {
        /// Provider ID to use for command suggestion generation.
        provider: ProviderId,
        /// Model ID to use for command suggestion generation.
        model: ModelId,
    },
    /// Set the reasoning effort level applied to all agents.
    ReasoningEffort {
        /// Effort level: none, minimal, low, medium, high, xhigh, max.
        effort: Effort,
    },
}

/// Type-safe subcommands for `forge config get`.
#[derive(Subcommand, Debug, Clone)]
pub enum ConfigGetField {
    /// Get the active model.
    Model,
    /// Get the active provider.
    Provider,
    /// Get the commit message generation config.
    Commit,
    /// Get the command suggestion generation config.
    Suggest,
    /// Get the reasoning effort level.
    ReasoningEffort,
}

/// Command group for conversation management.
#[derive(Parser, Debug, Clone)]
pub struct ConversationCommandGroup {
    #[command(subcommand)]
    pub command: ConversationCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConversationCommand {
    /// List conversation history.
    List {
        /// Output in machine-readable format.
        #[arg(long)]
        porcelain: bool,
    },

    /// Create a new conversation.
    New,

    /// Export conversation as JSON or HTML.
    Dump {
        /// Conversation ID to export.
        id: ConversationId,

        /// Export as HTML instead of JSON.
        #[arg(long)]
        html: bool,
    },

    /// Compact conversation to reduce token usage.
    Compact {
        /// Conversation ID to compact.
        id: ConversationId,
    },

    /// Retry last command without modifying context.
    Retry {
        /// Conversation ID to retry.
        id: ConversationId,
    },

    /// Resume conversation in interactive mode.
    Resume {
        /// Conversation ID to resume.
        id: ConversationId,
    },

    /// Show last assistant message.
    Show {
        /// Conversation ID.
        id: ConversationId,

        /// Print raw markdown without rendering.
        #[arg(long)]
        md: bool,
    },

    /// Show conversation details.
    Info {
        /// Conversation ID.
        id: ConversationId,
    },

    /// Show conversation statistics.
    Stats {
        /// Conversation ID.
        id: ConversationId,

        /// Output in machine-readable format.
        #[arg(long)]
        porcelain: bool,
    },

    /// Clone conversation with a new ID.
    Clone {
        /// Conversation ID to clone.
        id: ConversationId,

        /// Output in machine-readable format.
        #[arg(long)]
        porcelain: bool,
    },

    /// Delete a conversation permanently.
    Delete {
        /// Conversation ID to delete.
        id: String,
    },

    /// Rename a conversation.
    Rename {
        /// Conversation ID to rename.
        id: ConversationId,

        /// New name for the conversation.
        name: String,
    },
}

/// Command group for provider authentication management.
#[derive(Parser, Debug, Clone)]
pub struct ProviderCommandGroup {
    #[command(subcommand)]
    pub command: ProviderCommand,

    /// Output in machine-readable format.
    #[arg(long, global = true)]
    pub porcelain: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProviderCommand {
    /// Authenticate with an API provider.
    ///
    /// Shows an interactive menu when no provider name is specified.
    Login {
        /// Provider name to authenticate with.
        provider: Option<ProviderId>,
    },

    /// Remove provider credentials.
    ///
    /// Shows an interactive menu when no provider name is specified.
    Logout {
        /// Provider name to log out from.
        provider: Option<ProviderId>,
    },

    /// List available providers.
    List {
        /// Filter providers by type (e.g., llm, context_engine). Can be
        /// specified multiple times.
        #[arg(long = "type", short = 't')]
        types: Vec<forge_domain::ProviderType>,
    },
}

/// Group of Commit-related commands
#[derive(Parser, Debug, Clone)]
pub struct CommitCommandGroup {
    /// Preview the commit message without committing
    #[arg(long)]
    pub preview: bool,

    /// Maximum git diff size in bytes (default: 100k)
    ///
    /// Limits the size of the git diff sent to the AI model. Large diffs are
    /// truncated to save tokens and reduce API costs. Minimum value is 5000
    /// bytes.
    #[arg(long = "max-diff", default_value = "100000", value_parser = clap::builder::RangedI64ValueParser::<usize>::new().range(5000..))]
    pub max_diff_size: Option<usize>,

    /// Git diff content (used internally for piped input)
    ///
    /// This field is populated when diff content is piped to the commit
    /// command. Users typically don't set this directly; instead, they pipe
    /// diff content: `git diff | forge commit --preview`
    #[arg(skip)]
    pub diff: Option<String>,

    /// Additional text to customize the commit message
    ///
    /// Provide additional context or instructions for the AI to use when
    /// generating the commit message. Multiple words can be provided without
    /// quotes: `forge commit fix typo in readme`
    pub text: Vec<String>,
}

/// Group of Data-related commands
#[derive(Parser, Debug, Clone)]
pub struct DataCommandGroup {
    /// Path to JSONL file to process
    #[arg(long)]
    pub input: String,

    /// Path to JSON schema file for LLM tool definition
    #[arg(long)]
    pub schema: String,

    /// Path to Handlebars template file for system prompt
    #[arg(long)]
    pub system_prompt: Option<String>,

    /// Path to Handlebars template file for user prompt
    #[arg(long)]
    pub user_prompt: Option<String>,

    /// Maximum number of concurrent LLM requests
    #[arg(long, default_value = "10")]
    pub concurrency: usize,
}

impl From<DataCommandGroup> for forge_domain::DataGenerationParameters {
    fn from(value: DataCommandGroup) -> Self {
        Self {
            input: value.input.into(),
            schema: value.schema.into(),
            system_prompt: value.system_prompt.map(Into::into),
            user_prompt: value.user_prompt.map(Into::into),
            concurrency: value.concurrency,
        }
    }
}

/// VS Code integration commands.
#[derive(Subcommand, Debug, Clone)]
pub enum VscodeCommand {
    /// Install the Forge VS Code extension.
    InstallExtension,
}

/// Update command arguments.
#[derive(Parser, Debug, Clone)]
pub struct UpdateArgs {
    /// Skip the confirmation prompt when applying updates.
    #[arg(long, default_value_t = false)]
    pub no_confirm: bool,
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_data_command_group_conversion() {
        use std::path::PathBuf;

        let fixture = DataCommandGroup {
            input: "path/to/input.jsonl".to_string(),
            schema: "path/to/schema.json".to_string(),
            system_prompt: Some("system prompt".to_string()),
            user_prompt: None,
            concurrency: 5,
        };
        let actual: forge_domain::DataGenerationParameters = fixture.into();
        let expected = forge_domain::DataGenerationParameters {
            input: PathBuf::from("path/to/input.jsonl"),
            schema: PathBuf::from("path/to/schema.json"),
            system_prompt: Some(PathBuf::from("system prompt")),
            user_prompt: None,
            concurrency: 5,
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_commit_default_max_diff_size() {
        let fixture = Cli::parse_from(["forge", "commit", "--preview"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Commit(commit)) => commit.max_diff_size,
            _ => panic!("Expected Commit command"),
        };
        let expected = Some(100000);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_commit_custom_max_diff_size() {
        let fixture = Cli::parse_from(["forge", "commit", "--preview", "--max-diff", "50000"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Commit(commit)) => commit.max_diff_size,
            _ => panic!("Expected Commit command"),
        };
        let expected = Some(50000);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_config_set_with_provider_and_model() {
        let fixture = Cli::parse_from([
            "forge",
            "config",
            "set",
            "model",
            "anthropic",
            "claude-sonnet-4-20250514",
        ]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Config(config)) => match config.command {
                ConfigCommand::Set(args) => match args.field {
                    ConfigSetField::Model { provider, model } => {
                        Some((provider.to_string(), model.as_str().to_string()))
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        let expected = Some((
            "Anthropic".to_string(),
            "claude-sonnet-4-20250514".to_string(),
        ));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_config_list() {
        let fixture = Cli::parse_from(["forge", "config", "list"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Config(config)) => matches!(config.command, ConfigCommand::List),
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_config_get_specific_field() {
        let fixture = Cli::parse_from(["forge", "config", "get", "model"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Config(config)) => match config.command {
                ConfigCommand::Get(args) => matches!(args.field, ConfigGetField::Model),
                _ => panic!("Expected ConfigCommand::Get"),
            },
            _ => panic!("Expected TopLevelCommand::Config"),
        };
        assert!(actual);
    }

    #[test]
    fn test_config_set_commit_with_provider_and_model() {
        let fixture = Cli::parse_from([
            "forge",
            "config",
            "set",
            "commit",
            "anthropic",
            "claude-haiku-4-20250514",
        ]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Config(config)) => match config.command {
                ConfigCommand::Set(args) => match args.field {
                    ConfigSetField::Commit { provider, model } => {
                        Some((provider.to_string(), model.as_str().to_string()))
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        let expected = Some((
            "Anthropic".to_string(),
            "claude-haiku-4-20250514".to_string(),
        ));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_conversation_list() {
        let fixture = Cli::parse_from(["forge", "conversation", "list"]);
        let is_list = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => {
                matches!(conversation.command, ConversationCommand::List { .. })
            }
            _ => false,
        };
        assert_eq!(is_list, true);
    }

    #[test]
    fn test_session_alias_list() {
        let fixture = Cli::parse_from(["forge", "session", "list"]);
        let is_list = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => {
                matches!(conversation.command, ConversationCommand::List { .. })
            }
            _ => false,
        };
        assert_eq!(is_list, true);
    }

    #[test]
    fn test_agent_id_long_flag() {
        let fixture = Cli::parse_from(["forge", "--agent", "sage"]);
        assert_eq!(fixture.agent, Some(AgentId::new("sage")));
    }

    #[test]
    fn test_agent_id_short_alias() {
        let fixture = Cli::parse_from(["forge", "--aid", "muse"]);
        assert_eq!(fixture.agent, Some(AgentId::new("muse")));
    }

    #[test]
    fn test_agent_id_with_prompt() {
        let fixture = Cli::parse_from(["forge", "--agent", "forge", "-p", "test prompt"]);
        assert_eq!(fixture.agent, Some(AgentId::new("forge")));
        assert_eq!(fixture.prompt, Some("test prompt".to_string()));
    }

    #[test]
    fn test_agent_id_not_provided() {
        let fixture = Cli::parse_from(["forge"]);
        assert_eq!(fixture.agent, None);
    }

    #[test]
    fn test_conversation_dump_json_with_id() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "dump",
            "550e8400-e29b-41d4-a716-446655440000",
        ]);
        let (id, html) = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Dump { id, html } => (id, html),
                _ => (ConversationId::default(), true),
            },
            _ => (ConversationId::default(), true),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap()
        );
        assert_eq!(html, false); // JSON is default
    }

    #[test]
    fn test_conversation_dump_html_with_id() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "dump",
            "550e8400-e29b-41d4-a716-446655440001",
            "--html",
        ]);
        let (id, html) = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Dump { id, html } => (id, html),
                _ => (ConversationId::default(), false),
            },
            _ => (ConversationId::default(), false),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440001").unwrap()
        );
        assert_eq!(html, true);
    }

    #[test]
    fn test_conversation_retry_with_id() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "retry",
            "550e8400-e29b-41d4-a716-446655440002",
        ]);
        let id = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Retry { id } => id,
                _ => ConversationId::default(),
            },
            _ => ConversationId::default(),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440002").unwrap()
        );
    }

    #[test]
    fn test_conversation_compact_with_id() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "compact",
            "550e8400-e29b-41d4-a716-446655440003",
        ]);
        let id = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Compact { id } => id,
                _ => ConversationId::default(),
            },
            _ => ConversationId::default(),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440003").unwrap()
        );
    }

    #[test]
    fn test_conversation_last_with_id() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "show",
            "550e8400-e29b-41d4-a716-446655440004",
        ]);
        let (id, md) = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Show { id, md } => (id, md),
                _ => (ConversationId::default(), false),
            },
            _ => (ConversationId::default(), false),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440004").unwrap()
        );
        assert_eq!(md, false);
    }

    #[test]
    fn test_conversation_show_with_md_flag() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "show",
            "550e8400-e29b-41d4-a716-446655440004",
            "--md",
        ]);
        let (id, md) = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Show { id, md } => (id, md),
                _ => (ConversationId::default(), false),
            },
            _ => (ConversationId::default(), false),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440004").unwrap()
        );
        assert_eq!(md, true);
    }

    #[test]
    fn test_conversation_resume() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "resume",
            "550e8400-e29b-41d4-a716-446655440005",
        ]);
        let id = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Resume { id } => id,
                _ => ConversationId::default(),
            },
            _ => ConversationId::default(),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440005").unwrap()
        );
    }

    #[test]
    fn test_list_tools_command_with_agent() {
        let fixture = Cli::parse_from(["forge", "list", "tool", "sage"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => match list.command {
                ListCommand::Tool { agent } => agent,
                _ => AgentId::default(),
            },
            _ => AgentId::default(),
        };
        let expected = AgentId::new("sage");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_list_conversation_command() {
        let fixture = Cli::parse_from(["forge", "list", "conversation"]);
        let is_conversation_list = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => matches!(list.command, ListCommand::Conversation),
            _ => false,
        };
        assert_eq!(is_conversation_list, true);
    }

    #[test]
    fn test_list_session_alias_command() {
        let fixture = Cli::parse_from(["forge", "list", "session"]);
        let is_conversation_list = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => matches!(list.command, ListCommand::Conversation),
            _ => false,
        };
        assert_eq!(is_conversation_list, true);
    }

    #[test]
    fn test_list_command_without_custom_flag() {
        let fixture = Cli::parse_from(["forge", "list", "command"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => match list.command {
                ListCommand::Command { custom } => custom,
                _ => true,
            },
            _ => true,
        };
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_list_command_with_custom_flag() {
        let fixture = Cli::parse_from(["forge", "list", "command", "--custom"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => match list.command {
                ListCommand::Command { custom } => custom,
                _ => false,
            },
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cmd_list_with_custom_flag() {
        let fixture = Cli::parse_from(["forge", "cmd", "list", "--custom"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Cmd(cmd_group)) => match cmd_group.command {
                CmdCommand::List { custom } => custom,
                _ => false,
            },
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_command_list_with_custom_flag() {
        let fixture = Cli::parse_from(["forge", "command", "list", "--custom"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Cmd(cmd_group)) => match cmd_group.command {
                CmdCommand::List { custom } => custom,
                _ => false,
            },
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_info_command_without_porcelain() {
        let fixture = Cli::parse_from(["forge", "info"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Info { porcelain, .. }) => porcelain,
            _ => true,
        };
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_info_command_with_porcelain() {
        let fixture = Cli::parse_from(["forge", "info", "--porcelain"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Info { porcelain, .. }) => porcelain,
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_info_command_with_conversation_id() {
        let fixture = Cli::parse_from([
            "forge",
            "info",
            "--conversation-id",
            "550e8400-e29b-41d4-a716-446655440006",
        ]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Info { conversation_id, .. }) => conversation_id,
            _ => None,
        };
        let expected = Some(ConversationId::parse("550e8400-e29b-41d4-a716-446655440006").unwrap());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_info_command_with_cid_alias() {
        let fixture = Cli::parse_from([
            "forge",
            "info",
            "--cid",
            "550e8400-e29b-41d4-a716-446655440007",
        ]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Info { conversation_id, .. }) => conversation_id,
            _ => None,
        };
        let expected = Some(ConversationId::parse("550e8400-e29b-41d4-a716-446655440007").unwrap());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_info_command_with_conversation_id_and_porcelain() {
        let fixture = Cli::parse_from([
            "forge",
            "info",
            "--cid",
            "550e8400-e29b-41d4-a716-446655440008",
            "--porcelain",
        ]);
        let (conversation_id, porcelain) = match fixture.subcommands {
            Some(TopLevelCommand::Info { conversation_id, porcelain }) => {
                (conversation_id, porcelain)
            }
            _ => (None, false),
        };
        assert_eq!(
            conversation_id,
            Some(ConversationId::parse("550e8400-e29b-41d4-a716-446655440008").unwrap())
        );
        assert_eq!(porcelain, true);
    }

    #[test]
    fn test_list_agents_without_porcelain() {
        let fixture = Cli::parse_from(["forge", "list", "agents"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => list.porcelain,
            _ => true,
        };
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_list_agents_with_porcelain() {
        let fixture = Cli::parse_from(["forge", "list", "agents", "--porcelain"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => list.porcelain,
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_mcp_list_with_porcelain() {
        let fixture = Cli::parse_from(["forge", "mcp", "list", "--porcelain"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Mcp(mcp)) => mcp.porcelain,
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_conversation_list_with_porcelain() {
        let fixture = Cli::parse_from(["forge", "conversation", "list", "--porcelain"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::List { porcelain } => porcelain,
                _ => false,
            },
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_list_models_with_porcelain() {
        let fixture = Cli::parse_from(["forge", "list", "models", "--porcelain"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => list.porcelain,
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_config_list_with_porcelain() {
        let fixture = Cli::parse_from(["forge", "config", "list", "--porcelain"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Config(config)) => config.porcelain,
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_conversation_info_with_id() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "info",
            "550e8400-e29b-41d4-a716-446655440009",
        ]);
        let id = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Info { id } => id,
                _ => ConversationId::default(),
            },
            _ => ConversationId::default(),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440009").unwrap()
        );
    }

    #[test]
    fn test_conversation_stats_with_porcelain() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "stats",
            "550e8400-e29b-41d4-a716-446655440010",
            "--porcelain",
        ]);
        let (id, porcelain) = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Stats { id, porcelain } => (id, porcelain),
                _ => (ConversationId::default(), false),
            },
            _ => (ConversationId::default(), false),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440010").unwrap()
        );
        assert_eq!(porcelain, true);
    }

    #[test]
    fn test_prompt_command() {
        let fixture = Cli::parse_from(["forge", "zsh", "rprompt"]);
        let r_prompt = matches!(
            fixture.subcommands,
            Some(TopLevelCommand::Zsh(ZshCommandGroup::Rprompt))
        );
        assert!(r_prompt);
    }

    #[test]
    fn test_session_alias_dump() {
        let fixture = Cli::parse_from([
            "forge",
            "session",
            "dump",
            "550e8400-e29b-41d4-a716-446655440011",
        ]);
        let (id, html) = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Dump { id, html } => (id, html),
                _ => (ConversationId::default(), true),
            },
            _ => (ConversationId::default(), true),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440011").unwrap()
        );
        assert_eq!(html, false);
    }

    #[test]
    fn test_session_alias_retry() {
        let fixture = Cli::parse_from([
            "forge",
            "session",
            "retry",
            "550e8400-e29b-41d4-a716-446655440012",
        ]);
        let id = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Retry { id } => id,
                _ => ConversationId::default(),
            },
            _ => ConversationId::default(),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440012").unwrap()
        );
    }

    #[test]
    fn test_prompt_with_conversation_id() {
        let fixture = Cli::parse_from([
            "forge",
            "-p",
            "hello",
            "--conversation-id",
            "550e8400-e29b-41d4-a716-446655440000",
        ]);
        let actual = fixture.conversation_id;
        let expected = Some(ConversationId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_conversation_id_without_prompt() {
        let fixture = Cli::parse_from([
            "forge",
            "--conversation-id",
            "550e8400-e29b-41d4-a716-446655440000",
        ]);
        let actual = fixture.conversation_id;
        let expected = Some(ConversationId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_conversation_clone_with_id() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "clone",
            "550e8400-e29b-41d4-a716-446655440013",
        ]);
        let id = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Clone { id, .. } => id,
                _ => ConversationId::default(),
            },
            _ => ConversationId::default(),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440013").unwrap()
        );
    }

    #[test]
    fn test_conversation_clone_with_porcelain() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "clone",
            "550e8400-e29b-41d4-a716-446655440014",
            "--porcelain",
        ]);
        let (id, porcelain) = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => match conversation.command {
                ConversationCommand::Clone { id, porcelain } => (id, porcelain),
                _ => (ConversationId::default(), false),
            },
            _ => (ConversationId::default(), false),
        };
        assert_eq!(
            id,
            ConversationId::parse("550e8400-e29b-41d4-a716-446655440014").unwrap()
        );
        assert_eq!(porcelain, true);
    }

    #[test]
    fn test_cmd_command_with_args() {
        let fixture =
            Cli::parse_from(["forge", "cmd", "execute", "custom-command", "arg1", "arg2"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Cmd(run_group)) => match run_group.command {
                CmdCommand::Execute { commands } => commands.join(" "),
                _ => panic!("Expected Execute command"),
            },
            _ => panic!("Expected Cmd command"),
        };
        let expected = "custom-command arg1 arg2".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_is_interactive_without_flags() {
        let fixture = Cli::parse_from(["forge"]);
        let actual = fixture.is_interactive();
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_commit_with_custom_text() {
        let fixture = Cli::parse_from(["forge", "commit", "fix", "typo", "in", "readme"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Commit(commit)) => commit.text,
            _ => panic!("Expected Commit command"),
        };
        let expected = ["fix", "typo", "in", "readme"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<String>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_commit_without_custom_text() {
        let fixture = Cli::parse_from(["forge", "commit", "--preview"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Commit(commit)) => commit.text,
            _ => panic!("Expected Commit command"),
        };
        let expected: Vec<String> = vec![];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_commit_with_text_and_flags() {
        let fixture = Cli::parse_from([
            "forge",
            "commit",
            "--preview",
            "--max-diff",
            "50000",
            "update",
            "docs",
        ]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Commit(commit)) => {
                (commit.preview, commit.max_diff_size, commit.text)
            }
            _ => panic!("Expected Commit command"),
        };
        let expected = (
            true,
            Some(50000),
            ["update", "docs"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>(),
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_list_skill_command() {
        let fixture = Cli::parse_from(["forge", "list", "skill"]);
        let is_skill_list = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => matches!(list.command, ListCommand::Skill { .. }),
            _ => false,
        };
        assert_eq!(is_skill_list, true);
    }

    #[test]
    fn test_list_skills_alias_command() {
        let fixture = Cli::parse_from(["forge", "list", "skills"]);
        let is_skill_list = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => matches!(list.command, ListCommand::Skill { .. }),
            _ => false,
        };
        assert_eq!(is_skill_list, true);
    }

    #[test]
    fn test_conversation_delete_with_id() {
        let fixture = Cli::parse_from([
            "forge",
            "conversation",
            "delete",
            "550e8400-e29b-41d4-a716-446655440000",
        ]);
        let is_delete_with_id = match fixture.subcommands {
            Some(TopLevelCommand::Conversation(conversation)) => {
                matches!(conversation.command, ConversationCommand::Delete { id: _ })
            }
            _ => false,
        };
        assert_eq!(is_delete_with_id, true);
    }

    #[test]
    fn test_list_skill_with_porcelain() {
        let fixture = Cli::parse_from(["forge", "list", "skill", "--porcelain"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => list.porcelain,
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_prompt_with_leading_hyphen() {
        let fixture = Cli::parse_from(["forge", "-p", "- hi"]);
        assert_eq!(fixture.prompt, Some("- hi".to_string()));
    }

    #[test]
    fn test_prompt_with_hyphen_flag_like_value() {
        let fixture = Cli::parse_from(["forge", "-p", "-test"]);
        assert_eq!(fixture.prompt, Some("-test".to_string()));
    }

    #[test]
    fn test_prompt_with_double_hyphen() {
        let fixture = Cli::parse_from(["forge", "-p", "--something"]);
        assert_eq!(fixture.prompt, Some("--something".to_string()));
    }

    #[test]
    fn test_terminal_theme_zsh() {
        let fixture = Cli::parse_from(["forge", "zsh", "theme"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Zsh(terminal)) => {
                matches!(terminal, ZshCommandGroup::Theme)
            }
            _ => false,
        };
        assert_eq!(actual, true);
    }

    #[test]
    fn test_terminal_plugin_zsh() {
        let fixture = Cli::parse_from(["forge", "zsh", "plugin"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Zsh(terminal)) => {
                matches!(terminal, ZshCommandGroup::Plugin)
            }
            _ => false,
        };
        assert_eq!(actual, true);
    }

    #[test]
    fn test_zsh_doctor() {
        let fixture = Cli::parse_from(["forge", "zsh", "doctor"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Zsh(terminal)) => {
                matches!(terminal, ZshCommandGroup::Doctor)
            }
            _ => false,
        };
        assert_eq!(actual, true);
    }

    #[test]
    fn test_zsh_setup() {
        let fixture = Cli::parse_from(["forge", "zsh", "setup"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Zsh(terminal)) => {
                matches!(terminal, ZshCommandGroup::Setup)
            }
            _ => false,
        };
        assert_eq!(actual, true);
    }

    #[test]
    fn test_zsh_keyboard() {
        let fixture = Cli::parse_from(["forge", "zsh", "keyboard"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Zsh(terminal)) => {
                matches!(terminal, ZshCommandGroup::Keyboard)
            }
            _ => false,
        };
        assert_eq!(actual, true);
    }

    #[test]
    fn test_zsh_format() {
        let fixture = Cli::parse_from(["forge", "zsh", "format", "--buffer", "hello world"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Zsh(ZshCommandGroup::Format { buffer })) => {
                buffer == "hello world"
            }
            _ => false,
        };
        assert_eq!(actual, true);
    }

    #[test]
    fn test_setup_alias() {
        let fixture = Cli::parse_from(["forge", "setup"]);
        let actual = matches!(fixture.subcommands, Some(TopLevelCommand::Setup));
        assert_eq!(actual, true);
    }

    #[test]
    fn test_doctor_alias() {
        let fixture = Cli::parse_from(["forge", "doctor"]);
        let actual = matches!(fixture.subcommands, Some(TopLevelCommand::Doctor));
        assert_eq!(actual, true);
    }

    #[test]
    fn test_install_vscode_extension() {
        let fixture = Cli::parse_from(["forge", "vscode", "install-extension"]);
        let actual = matches!(
            fixture.subcommands,
            Some(TopLevelCommand::Vscode(VscodeCommand::InstallExtension))
        );
        assert_eq!(actual, true);
    }

    #[test]
    fn test_list_agent_with_custom_flag() {
        let fixture = Cli::parse_from(["forge", "list", "agent", "--custom"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => match list.command {
                ListCommand::Agent { custom } => custom,
                _ => false,
            },
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_list_agent_without_custom_flag() {
        let fixture = Cli::parse_from(["forge", "list", "agent"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => match list.command {
                ListCommand::Agent { custom } => custom,
                _ => true,
            },
            _ => true,
        };
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_list_skill_with_custom_flag() {
        let fixture = Cli::parse_from(["forge", "list", "skill", "--custom"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => match list.command {
                ListCommand::Skill { custom } => custom,
                _ => false,
            },
            _ => false,
        };
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_list_skill_without_custom_flag() {
        let fixture = Cli::parse_from(["forge", "list", "skill"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::List(list)) => match list.command {
                ListCommand::Skill { custom } => custom,
                _ => true,
            },
            _ => true,
        };
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_update_with_no_confirm() {
        let fixture = Cli::parse_from(["forge", "update", "--no-confirm"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Update(args)) => args.no_confirm,
            _ => panic!("Expected Update command"),
        };
        assert!(actual);
    }

    #[test]
    fn test_update_without_no_confirm() {
        let fixture = Cli::parse_from(["forge", "update"]);
        let actual = match fixture.subcommands {
            Some(TopLevelCommand::Update(args)) => args.no_confirm,
            _ => panic!("Expected Update command"),
        };
        assert!(!actual);
    }
}
