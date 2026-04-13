use derive_setters::Setters;
use serde::Deserialize;

/// A user-defined command loaded from a Markdown file with YAML frontmatter.
///
/// Commands are discovered from `.md` files in the forge commands directories
/// and made available as slash commands in the UI. The `name` and `description`
/// come from YAML frontmatter; the `prompt` is the Markdown body of the file.
#[derive(Debug, Clone, Default, Deserialize, Setters, PartialEq)]
#[setters(into, strip_option)]
pub struct Command {
    /// The command name used to invoke it (e.g. `github-pr-description`).
    #[serde(default)]
    pub name: String,
    /// Short description shown in the command list.
    #[serde(default)]
    pub description: String,
    /// The prompt template body (Markdown content after the frontmatter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}
