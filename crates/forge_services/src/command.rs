use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use forge_app::domain::Command;
use forge_app::{
    DirectoryReaderInfra, EnvironmentInfra, FileInfoInfra, FileReaderInfra, FileWriterInfra,
};
use gray_matter::Matter;
use gray_matter::engine::YAML;

pub struct CommandLoaderService<F> {
    infra: Arc<F>,

    // Cache is used to maintain the loaded commands
    // for this service instance.
    // So that they could live till user starts a new session.
    cache: tokio::sync::OnceCell<Vec<Command>>,
}

impl<F> CommandLoaderService<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra, cache: Default::default() }
    }

    /// Load built-in commands that are embedded in the application binary.
    fn init_default(&self) -> anyhow::Result<Vec<Command>> {
        parse_command_iter(
            [(
                "github-pr-description",
                include_str!("../../../commands/github-pr-description.md"),
            )]
            .into_iter()
            .map(|(name, content)| (name.to_string(), content.to_string())),
        )
    }
}

#[async_trait::async_trait]
impl<F: FileReaderInfra + FileWriterInfra + FileInfoInfra + EnvironmentInfra + DirectoryReaderInfra>
    forge_app::CommandLoaderService for CommandLoaderService<F>
{
    async fn get_commands(&self) -> anyhow::Result<Vec<Command>> {
        self.cache_or_init().await
    }
}

impl<F: FileReaderInfra + FileWriterInfra + FileInfoInfra + EnvironmentInfra + DirectoryReaderInfra>
    CommandLoaderService<F>
{
    /// Load all command definitions with caching support
    async fn cache_or_init(&self) -> anyhow::Result<Vec<Command>> {
        self.cache.get_or_try_init(|| self.init()).await.cloned()
    }

    async fn init(&self) -> anyhow::Result<Vec<Command>> {
        // Load built-in commands first (lowest precedence)
        let mut commands = self.init_default()?;

        // Load custom commands from global directory
        let dir = self.infra.get_environment().command_path();
        let custom_commands = self.init_command_dir(&dir).await?;
        commands.extend(custom_commands);

        // Load custom commands from CWD
        let dir = self.infra.get_environment().command_path_local();
        let cwd_commands = self.init_command_dir(&dir).await?;

        commands.extend(cwd_commands);

        // Handle command name conflicts by keeping the last occurrence
        // This gives precedence order: CWD > Global Custom > Built-in
        Ok(resolve_command_conflicts(commands))
    }

    async fn init_command_dir(&self, dir: &std::path::Path) -> anyhow::Result<Vec<Command>> {
        if !self.infra.exists(dir).await? {
            return Ok(vec![]);
        }

        // Use DirectoryReaderInfra to read all .md files in parallel
        let files = self
            .infra
            .read_directory_files(dir, Some("*.md"))
            .await
            .with_context(|| format!("Failed to read commands from: {}", dir.display()))?;

        parse_command_iter(files.into_iter().map(|(path, content)| {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            (name, content)
        }))
    }
}

/// Implementation function for resolving command name conflicts by keeping the
/// last occurrence. This implements the precedence order: CWD Custom > Global
/// Custom
/// > Built-in
fn resolve_command_conflicts(commands: Vec<Command>) -> Vec<Command> {
    // Use HashMap to deduplicate by command name, keeping the last occurrence
    let mut command_map: HashMap<String, Command> = HashMap::new();

    for command in commands {
        command_map.insert(command.name.clone(), command);
    }

    // Convert back to vector (order is not guaranteed but doesn't matter for the
    // service)
    command_map.into_values().collect()
}

fn parse_command_iter<I, Path: AsRef<str>, Content: AsRef<str>>(
    contents: I,
) -> anyhow::Result<Vec<Command>>
where
    I: Iterator<Item = (Path, Content)>,
{
    let mut commands = vec![];

    for (name, content) in contents {
        let command = parse_command_file(content.as_ref())
            .with_context(|| format!("Failed to parse command: {}", name.as_ref()))?;

        commands.push(command);
    }

    Ok(commands)
}

/// Parse raw content into a Command with YAML frontmatter
fn parse_command_file(content: &str) -> Result<Command> {
    // Parse the frontmatter using gray_matter with type-safe deserialization
    let gray_matter = Matter::<YAML>::new();
    let result = gray_matter.parse::<Command>(content)?;

    // Extract the frontmatter
    let command = result
        .data
        .context("Empty command frontmatter")?
        .prompt(result.content);

    Ok(command)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[tokio::test]
    async fn test_parse_basic_command() {
        let content = forge_test_kit::fixture!("src/fixtures/commands/basic.md").await;

        let actual = parse_command_file(&content).unwrap();

        assert_eq!(actual.name.as_str(), "test-basic");
        assert_eq!(actual.description.as_str(), "A basic test command");
        assert_eq!(
            actual.prompt.as_ref().unwrap(),
            "This is the prompt content for the basic test command."
        );
    }

    #[tokio::test]
    async fn test_parse_command_with_multiline_prompt() {
        let content = forge_test_kit::fixture!("src/fixtures/commands/multiline.md").await;

        let actual = parse_command_file(&content).unwrap();

        assert_eq!(actual.name.as_str(), "test-multiline");
        assert_eq!(actual.description.as_str(), "Command with multiline prompt");
        assert!(actual.prompt.as_ref().unwrap().contains("Step 1"));
        assert!(actual.prompt.as_ref().unwrap().contains("Step 2"));
    }

    #[tokio::test]
    async fn test_parse_invalid_frontmatter() {
        let content = forge_test_kit::fixture!("src/fixtures/commands/invalid.md").await;

        let result = parse_command_file(&content);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_parse_builtin_commands() {
        // Test that all built-in commands parse correctly
        let builtin_commands = [
            ("fixme", "../../.forge/commands/fixme.md"),
            ("check", "../../.forge/commands/check.md"),
        ];

        for (name, path) in builtin_commands {
            let content = forge_test_kit::fixture!(path).await;
            let command = parse_command_file(&content)
                .with_context(|| format!("Failed to parse built-in command: {}", name))
                .unwrap();

            assert_eq!(command.name.as_str(), name);
            assert!(!command.description.is_empty());
            assert!(command.prompt.is_some());
        }
    }

    #[test]
    fn test_init_default_contains_builtin_commands() {
        // Fixture
        let service = CommandLoaderService::<()> { infra: Arc::new(()), cache: Default::default() };

        // Execute
        let actual = service.init_default().unwrap();

        // Verify github-pr-description
        let command = actual
            .iter()
            .find(|c| c.name.as_str() == "github-pr-description")
            .expect("github-pr-description should be a built-in command");

        assert_eq!(command.name.as_str(), "github-pr-description");
        assert!(!command.description.is_empty());
        assert!(command.prompt.is_some());
    }

    #[test]
    fn test_resolve_command_conflicts_no_duplicates() {
        let fixture = vec![
            Command::default().name("command1").description("Command 1"),
            Command::default().name("command2").description("Command 2"),
            Command::default().name("command3").description("Command 3"),
        ];

        let actual = resolve_command_conflicts(fixture.clone());

        // Should return all commands when no conflicts
        assert_eq!(actual.len(), 3);

        let names: std::collections::HashSet<_> = actual.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains("command1"));
        assert!(names.contains("command2"));
        assert!(names.contains("command3"));
    }

    #[test]
    fn test_resolve_command_conflicts_with_duplicates() {
        let fixture = vec![
            Command::default()
                .name("command1")
                .description("Global Command 1"),
            Command::default()
                .name("command2")
                .description("Global Command 2"),
            Command::default()
                .name("command1")
                .description("CWD Command 1 - Override"), // Duplicate name, should override
            Command::default()
                .name("command3")
                .description("CWD Command 3"),
        ];

        let actual = resolve_command_conflicts(fixture);

        // Should have 3 commands: command1 (CWD version), command2 (global), command3
        // (CWD)
        assert_eq!(actual.len(), 3);

        let command1 = actual
            .iter()
            .find(|c| c.name.as_str() == "command1")
            .expect("Should have command1");
        let expected_description = "CWD Command 1 - Override";
        assert_eq!(command1.description.as_str(), expected_description);
    }

    #[test]
    fn test_resolve_command_conflicts_multiple_duplicates() {
        // Test scenario: Built-in -> Global -> CWD (CWD should win)
        let fixture = vec![
            Command::default()
                .name("common")
                .description("Built-in Common Command"),
            Command::default()
                .name("unique1")
                .description("Built-in Unique 1"),
            Command::default()
                .name("common")
                .description("Global Common Command"), // Override built-in
            Command::default()
                .name("unique2")
                .description("Global Unique 2"),
            Command::default()
                .name("common")
                .description("CWD Common Command"), // Override global
            Command::default()
                .name("unique3")
                .description("CWD Unique 3"),
        ];

        let actual = resolve_command_conflicts(fixture);

        // Should have 4 commands: common (CWD version), unique1, unique2, unique3
        assert_eq!(actual.len(), 4);

        let common = actual
            .iter()
            .find(|c| c.name.as_str() == "common")
            .expect("Should have common command");
        let expected_description = "CWD Common Command";
        assert_eq!(common.description.as_str(), expected_description);

        // Verify all unique commands are present
        let names: std::collections::HashSet<_> = actual.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains("common"));
        assert!(names.contains("unique1"));
        assert!(names.contains("unique2"));
        assert!(names.contains("unique3"));
    }

    #[test]
    fn test_resolve_command_conflicts_empty_input() {
        let fixture: Vec<Command> = vec![];

        let actual = resolve_command_conflicts(fixture);

        assert_eq!(actual.len(), 0);
    }
}
