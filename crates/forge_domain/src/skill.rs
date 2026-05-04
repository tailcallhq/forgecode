use std::path::PathBuf;

use derive_setters::Setters;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Represents a reusable skill with a name, file path, and prompt content
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Setters, JsonSchema)]
#[setters(strip_option, into)]
pub struct Skill {
    /// Name of the skill
    pub name: String,

    /// File path to the skill markdown file
    pub path: Option<PathBuf>,

    /// Content/prompt loaded from the markdown file
    pub command: String,

    /// Description of the skill
    pub description: String,

    /// List of positional argument names declared in skill frontmatter
    pub arguments: Vec<String>,

    /// List of resource files in the skill directory
    pub resources: Vec<PathBuf>,
}

impl Skill {
    /// Creates a new Skill with required fields
    ///
    /// # Arguments
    ///
    /// * `name` - The name identifier for the skill
    /// * `prompt` - The skill prompt content
    /// * `description` - A brief description of the skill
    pub fn new(
        name: impl Into<String>,
        prompt: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            path: None,
            command: prompt.into(),
            description: description.into(),
            arguments: Vec::new(),
            resources: Vec::new(),
        }
    }

    /// Applies invocation arguments to skill placeholders.
    ///
    /// Replaces `$ARGUMENTS` with the full argument string, indexed
    /// placeholders (`$ARGUMENTS[0]`, `$0`, etc.) with shell-style parsed
    /// arguments, and named placeholders (`$plan_path`) using frontmatter
    /// argument names from `Skill.arguments`. Missing arguments are replaced
    /// with empty strings.
    ///
    /// # Arguments
    ///
    /// * `arguments` - Optional raw argument string passed during skill
    ///   invocation
    #[must_use]
    pub fn with_arguments(self, arguments: Option<&str>) -> Self {
        let command_contains_full_arguments = self.command.contains("$ARGUMENTS");
        let argument_text = arguments.unwrap_or_default().trim();
        let parsed_arguments = parse_skill_arguments(argument_text);

        let indexed_pattern =
            Regex::new(r"\$ARGUMENTS\[(\d+)\]").expect("Indexed argument regex should be valid");
        let shorthand_pattern =
            Regex::new(r"\$(\d+)").expect("Positional argument regex should be valid");

        let mut rendered = indexed_pattern
            .replace_all(&self.command, |captures: &regex::Captures<'_>| {
                let idx = captures
                    .get(1)
                    .and_then(|m| m.as_str().parse::<usize>().ok())
                    .unwrap_or_default();

                parsed_arguments.get(idx).cloned().unwrap_or_default()
            })
            .into_owned();

        rendered = shorthand_pattern
            .replace_all(&rendered, |captures: &regex::Captures<'_>| {
                let idx = captures
                    .get(1)
                    .and_then(|m| m.as_str().parse::<usize>().ok())
                    .unwrap_or_default();

                parsed_arguments.get(idx).cloned().unwrap_or_default()
            })
            .into_owned();

        for (idx, argument_name) in self.arguments.iter().enumerate() {
            let named_pattern = Regex::new(&format!(r"\${}\b", regex::escape(argument_name)))
                .expect("Named argument regex should be valid");
            rendered = named_pattern
                .replace_all(
                    &rendered,
                    parsed_arguments
                        .get(idx)
                        .map(String::as_str)
                        .unwrap_or_default(),
                )
                .into_owned();
        }

        rendered = rendered.replace("$ARGUMENTS", argument_text);

        if !argument_text.is_empty() && !command_contains_full_arguments {
            rendered = format!("{rendered}\n\nARGUMENTS: {argument_text}");
        }

        Self { command: rendered, ..self }
    }
}

fn parse_skill_arguments(arguments: &str) -> Vec<String> {
    if arguments.is_empty() {
        return vec![];
    }

    shell_words::split(arguments).unwrap_or_else(|_| {
        arguments
            .split_whitespace()
            .map(std::string::ToString::to_string)
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_skill_creation() {
        // Fixture
        let fixture = Skill::new(
            "code_review",
            "Review this code",
            "A skill for reviewing code quality",
        )
        .path("/skills/code_review.md");

        // Act
        let actual = (
            fixture.name.clone(),
            fixture.path.clone(),
            fixture.command.clone(),
            fixture.description.clone(),
        );

        // Assert
        let expected = (
            "code_review".to_string(),
            Some("/skills/code_review.md".into()),
            "Review this code".to_string(),
            "A skill for reviewing code quality".to_string(),
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_skill_with_setters() {
        // Fixture
        let fixture = Skill::new("test", "prompt", "desc")
            .path("/path")
            .name("updated_name")
            .path("/updated/path")
            .command("updated prompt")
            .description("updated description");

        // Act
        let actual = fixture;

        // Assert
        let expected = Skill::new("updated_name", "updated prompt", "updated description")
            .path("/updated/path");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_skill_with_arguments_renders_full_and_positional_placeholders() {
        // Fixture
        let fixture = Skill::new(
            "pdf",
            "Run with all args: $ARGUMENTS\nfirst=$0\nsecond=$1\nthird=$ARGUMENTS[2]",
            "desc",
        )
        .arguments(vec![
            "action".to_string(),
            "first_file".to_string(),
            "second_file".to_string(),
        ]);

        // Act
        let actual = fixture.with_arguments(Some("merge \"a file.pdf\" b.pdf"));

        // Assert
        let expected = Skill::new(
            "pdf",
            "Run with all args: merge \"a file.pdf\" b.pdf\nfirst=merge\nsecond=a file.pdf\nthird=b.pdf",
            "desc",
        )
        .arguments(vec![
            "action".to_string(),
            "first_file".to_string(),
            "second_file".to_string(),
        ]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_skill_with_arguments_renders_named_placeholders_from_frontmatter() {
        // Fixture
        let fixture = Skill::new("execute-plan", "path=$plan_path\nmode=$mode", "desc")
            .arguments(vec!["plan_path".to_string(), "mode".to_string()]);

        // Act
        let actual = fixture.with_arguments(Some("plans/2026-04-28-upgrade-v1.md strict"));

        // Assert
        let expected = Skill::new(
            "execute-plan",
            "path=plans/2026-04-28-upgrade-v1.md\nmode=strict\n\nARGUMENTS: plans/2026-04-28-upgrade-v1.md strict",
            "desc",
        )
        .arguments(vec!["plan_path".to_string(), "mode".to_string()]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_skill_with_arguments_appends_arguments_when_full_placeholder_missing() {
        // Fixture
        let fixture = Skill::new("pdf", "first=$0", "desc");

        // Act
        let actual = fixture.with_arguments(Some("merge input.pdf"));

        // Assert
        let expected = Skill::new("pdf", "first=merge\n\nARGUMENTS: merge input.pdf", "desc");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_skill_with_arguments_defaults_to_empty_when_not_provided() {
        // Fixture
        let fixture = Skill::new("pdf", "args=$ARGUMENTS\nzero=$0\none=$1", "desc");

        // Act
        let actual = fixture.with_arguments(None);

        // Assert
        let expected = Skill::new("pdf", "args=\nzero=\none=", "desc");
        assert_eq!(actual, expected);
    }
}
