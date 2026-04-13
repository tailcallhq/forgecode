use std::sync::{Arc, Mutex};

use forge_api::{AgentInfo, Model, Template};
use forge_domain::UserCommand;
use strum::{EnumProperty, IntoEnumIterator};
use strum_macros::{EnumIter, EnumProperty};

use crate::info::Info;

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
        )
    }

    fn default_commands() -> Vec<ForgeCommand> {
        SlashCommand::iter()
            .filter(|command| !matches!(command, SlashCommand::Message(_)))
            .filter(|command| !matches!(command, SlashCommand::Custom(_)))
            .filter(|command| !matches!(command, SlashCommand::Shell(_)))
            .filter(|command| !matches!(command, SlashCommand::AgentSwitch(_)))
            .filter(|command| !matches!(command, SlashCommand::Rename(_)))
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

    pub fn parse(&self, input: &str) -> anyhow::Result<SlashCommand> {
        // Check if it's a shell command (starts with !)
        if input.trim().starts_with("!") {
            return Ok(SlashCommand::Shell(
                input
                    .strip_prefix("!")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            ));
        }

        let mut tokens = input.trim().split_ascii_whitespace();
        let command = tokens.next().unwrap();
        let parameters = tokens.collect::<Vec<_>>();

        // Check if it's a system command (starts with /)
        let is_command = command.starts_with("/");
        if !is_command {
            return Ok(SlashCommand::Message(input.to_string()));
        }

        // TODO: Can leverage Clap to parse commands and provide correct error messages
        match command {
            "/compact" => Ok(SlashCommand::Compact),
            "/new" => Ok(SlashCommand::New),
            "/info" => Ok(SlashCommand::Info),
            "/usage" => Ok(SlashCommand::Usage),
            "/exit" => Ok(SlashCommand::Exit),
            "/update" => Ok(SlashCommand::Update),
            "/dump" => {
                let html = !parameters.is_empty() && parameters[0] == "html";
                Ok(SlashCommand::Dump { html })
            }
            "/act" | "/forge" => Ok(SlashCommand::Forge),
            "/plan" | "/muse" => Ok(SlashCommand::Muse),
            "/sage" => Ok(SlashCommand::Sage),
            "/help" => Ok(SlashCommand::Help),
            "/model" => Ok(SlashCommand::Model),
            "/provider" | "/login" => Ok(SlashCommand::Login),
            "/tools" => Ok(SlashCommand::Tools),
            "/agent" => Ok(SlashCommand::Agent),
            "/logout" => Ok(SlashCommand::Logout),
            "/retry" => Ok(SlashCommand::Retry),
            "/conversation" | "/conversations" => Ok(SlashCommand::Conversations),
            "/commit" => {
                // Support flexible syntax:
                // /commit              -> commit with AI message
                // /commit 5000         -> commit with max-diff of 5000 bytes
                let max_diff_size = parameters.iter().find_map(|&p| p.parse::<usize>().ok());
                Ok(SlashCommand::Commit { max_diff_size })
            }
            "/index" => Ok(SlashCommand::Index),
            "/rename" | "/rn" => {
                let name = parameters.join(" ");
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Err(anyhow::anyhow!(
                        "Usage: /rename <name>. Please provide a name for the conversation."
                    ));
                }
                Ok(SlashCommand::Rename(name))
            }
            text => {
                let parts = text.split_ascii_whitespace().collect::<Vec<&str>>();

                if let Some(command) = parts.first() {
                    // Check if it's an agent command pattern (/agent-*)
                    if command.starts_with("/agent-") {
                        let command_name = command.strip_prefix('/').unwrap();
                        if let Some(found_command) = self.find(command_name) {
                            // Extract the agent ID from the command value
                            if let Some(agent_id) = &found_command.value {
                                return Ok(SlashCommand::AgentSwitch(agent_id.clone()));
                            }
                        }
                        return Err(anyhow::anyhow!("{command} is not a valid agent command"));
                    }

                    // Handle custom workflow commands
                    let command_name = command.strip_prefix('/').unwrap_or(command);
                    if let Some(command) = self.find(command_name) {
                        let template = Template::new(
                            self.extract_command_value(&command, &parts[1..])
                                .unwrap_or_default(),
                        );
                        Ok(SlashCommand::Custom(UserCommand::new(
                            command.name.clone(),
                            template,
                            parameters.into_iter().map(|s| s.to_owned()).collect(),
                        )))
                    } else {
                        Err(anyhow::anyhow!("{command} is not valid"))
                    }
                } else {
                    Err(anyhow::anyhow!("Invalid Command Format."))
                }
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
#[derive(Debug, Clone, PartialEq, Eq, EnumProperty, EnumIter)]
pub enum SlashCommand {
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
    /// This can be triggered with the '/forge' command.
    #[strum(props(usage = "Enable implementation mode with code changes"))]
    Forge,
    /// Switch to "muse" agent.
    /// This can be triggered with the '/must' command.
    #[strum(props(usage = "Enable planning mode without code changes"))]
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
    Help,
    /// Dumps the current conversation into a json file or html file
    #[strum(props(usage = "Save conversation as JSON or HTML (use /dump --html for HTML format)"))]
    Dump { html: bool },
    /// Switch or select the active model
    /// This can be triggered with the '/model' command.
    #[strum(props(usage = "Switch to a different model"))]
    Model,
    /// List all available tools with their descriptions and schema
    /// This can be triggered with the '/tools' command.
    #[strum(props(usage = "List all available tools with their descriptions and schema"))]
    Tools,
    /// Handles custom command defined in workflow file.
    Custom(UserCommand),
    /// Executes a native shell command.
    /// This can be triggered with commands starting with '!' character.
    #[strum(props(usage = "Execute a native shell command"))]
    Shell(String),

    /// Allows user to switch the operating agent.
    #[strum(props(usage = "Switch to an agent interactively"))]
    Agent,

    /// Allows you to configure provider
    #[strum(props(usage = "Allows you to configure provider"))]
    Login,

    /// Logs out from the configured provider
    #[strum(props(usage = "Logout from configured provider"))]
    Logout,

    /// Retry without modifying model context
    #[strum(props(usage = "Retry the last command"))]
    Retry,
    /// List all conversations for the active workspace
    #[strum(props(usage = "List all conversations for the active workspace"))]
    Conversations,

    /// Delete a conversation permanently
    #[strum(props(usage = "Delete a conversation permanently"))]
    Delete,

    /// Rename the current conversation
    #[strum(props(usage = "Rename the current conversation. Usage: /rename <name>"))]
    Rename(String),

    /// Switch directly to a specific agent by ID
    #[strum(props(usage = "Switch directly to a specific agent"))]
    AgentSwitch(String),

    /// Generate and optionally commit changes with AI-generated message
    ///
    /// Examples:
    /// - `/commit` - Generate message and commit
    /// - `/commit 5000` - Commit with max diff of 5000 bytes
    #[strum(props(
        usage = "Generate AI commit message and commit changes. Format: /commit <max-diff|preview>"
    ))]
    Commit { max_diff_size: Option<usize> },

    /// Index the current workspace for semantic code search
    #[strum(props(usage = "Index the current workspace for semantic search"))]
    Index,
}

impl SlashCommand {
    pub fn name(&self) -> &str {
        match self {
            SlashCommand::Compact => "compact",
            SlashCommand::New => "new",
            SlashCommand::Message(_) => "message",
            SlashCommand::Update => "update",
            SlashCommand::Info => "info",
            SlashCommand::Usage => "usage",
            SlashCommand::Exit => "exit",
            SlashCommand::Forge => "forge",
            SlashCommand::Muse => "muse",
            SlashCommand::Sage => "sage",
            SlashCommand::Help => "help",
            SlashCommand::Commit { .. } => "commit",
            SlashCommand::Dump { .. } => "dump",
            SlashCommand::Model => "model",
            SlashCommand::Tools => "tools",
            SlashCommand::Custom(event) => &event.name,
            SlashCommand::Shell(_) => "!shell",
            SlashCommand::Agent => "agent",
            SlashCommand::Login => "login",
            SlashCommand::Logout => "logout",
            SlashCommand::Retry => "retry",
            SlashCommand::Conversations => "conversation",
            SlashCommand::Delete => "delete",
            SlashCommand::Rename(_) => "rename",
            SlashCommand::AgentSwitch(agent_id) => agent_id,
            SlashCommand::Index => "index",
        }
    }

    /// Returns the usage description for the command.
    pub fn usage(&self) -> &str {
        self.get_str("usage").unwrap()
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
            SlashCommand::Shell(cmd) => assert_eq!(cmd, "ls -la"),
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
            SlashCommand::Shell(cmd) => assert_eq!(cmd, ""),
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
            SlashCommand::Shell(cmd) => assert_eq!(cmd, "echo 'test'"),
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
            SlashCommand::Conversations => {
                // Command parsed correctly
            }
            _ => panic!("Expected List command, got {result:?}"),
        }
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
            SlashCommand::AgentSwitch(agent_id) => assert_eq!(agent_id, "test-agent"),
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
            SlashCommand::Commit { max_diff_size } => {
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
            SlashCommand::Commit { max_diff_size } => {
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
            SlashCommand::Commit { max_diff_size } => {
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
            SlashCommand::Commit { max_diff_size } => {
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
    fn test_parse_tool_command() {
        // Setup
        let fixture = ForgeCommandManager::default();

        // Execute
        let result = fixture.parse("/tools").unwrap();

        // Verify
        match result {
            SlashCommand::Tools => {
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
        let expected = SlashCommand::Dump { html: false };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_dump_command_html_without_dashes() {
        // Setup
        let fixture = ForgeCommandManager::default();

        // Execute
        let actual = fixture.parse("/dump html").unwrap();

        // Verify
        let expected = SlashCommand::Dump { html: true };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_rename_command() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/rename my-session").unwrap();
        assert_eq!(actual, SlashCommand::Rename("my-session".to_string()));
    }

    #[test]
    fn test_parse_rename_command_multi_word() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/rename auth refactor work").unwrap();
        assert_eq!(
            actual,
            SlashCommand::Rename("auth refactor work".to_string())
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
        assert_eq!(actual, SlashCommand::Rename("my-session".to_string()));
    }

    #[test]
    fn test_parse_rename_trims_whitespace() {
        let fixture = ForgeCommandManager::default();
        let actual = fixture.parse("/rename   my title   ").unwrap();
        assert_eq!(actual, SlashCommand::Rename("my title".to_string()));
    }

    #[test]
    fn test_rename_is_reserved_command() {
        assert!(ForgeCommandManager::is_reserved_command("rename"));
        assert!(ForgeCommandManager::is_reserved_command("rn"));
    }

    #[test]
    fn test_rename_command_name() {
        let cmd = SlashCommand::Rename("test".to_string());
        assert_eq!(cmd.name(), "rename");
    }
}
