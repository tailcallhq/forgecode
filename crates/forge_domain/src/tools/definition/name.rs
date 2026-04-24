use std::fmt::Display;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ToolName(String);

impl ToolName {
    pub fn new(value: impl ToString) -> Self {
        ToolName(value.to_string())
    }

    /// Transforms the tool_name to remove whitespaces and converts to
    /// lower_snake_case
    pub fn sanitized(input: &str) -> Self {
        // Convert to lowercase
        let input = input.to_lowercase();

        // Replace all non-alphanumeric characters (excluding underscore) with
        // underscores
        let re_special = Regex::new(r"[^a-z0-9_]+").unwrap();
        let cleaned = re_special.replace_all(&input, "_");

        // Remove leading/trailing underscores and collapse consecutive underscores
        let re_trimmed = Regex::new(r"_+").unwrap();

        let sanitized_str = re_trimmed
            .replace_all(&cleaned, "_")
            .trim_matches('_')
            .to_string();

        Self(sanitized_str)
    }
}

impl ToolName {
    pub fn into_string(self) -> String {
        self.0
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_sanitized(self) -> Self {
        ToolName::sanitized(self.0.as_str())
    }

    /// Converts a Claude Code format MCP tool name (`mcp__{server}__{tool}`) to
    /// Forge's internal legacy format (`mcp_{server}_tool_{tool}`).
    ///
    /// Returns `None` if the name does not match the Claude Code MCP format.
    /// Sanitized names never contain `__`, so the first `__` is always the
    /// server/tool separator.
    pub fn to_legacy_mcp_name(&self) -> Option<ToolName> {
        let rest = self.0.strip_prefix("mcp__")?;
        let (server, tool) = rest.split_once("__")?;
        Some(ToolName::new(format!("mcp_{server}_tool_{tool}")))
    }
}

impl From<String> for ToolName {
    fn from(value: String) -> Self {
        ToolName::new(value)
    }
}

impl From<&str> for ToolName {
    fn from(value: &str) -> Self {
        ToolName::new(value)
    }
}

pub trait NamedTool {
    fn tool_name() -> ToolName;
}

impl Display for ToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_sanitize_camel_case() {
        let tool_name = ToolName::new("camelCase");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("camelcase");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_pascal_case() {
        let tool_name = ToolName::new("PascalCase");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("pascalcase");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_mixed_case_with_numbers() {
        let tool_name = ToolName::new("myTool2Name");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("mytool2name");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_special_characters() {
        let tool_name = ToolName::new("tool-name@with#special$chars");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("tool_name_with_special_chars");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_whitespace() {
        let tool_name = ToolName::new("tool name with spaces");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("tool_name_with_spaces");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_consecutive_special_chars() {
        let tool_name = ToolName::new("tool---name___with@@@special");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("tool_name_with_special");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_leading_trailing_special_chars() {
        let tool_name = ToolName::new("___tool_name___");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("tool_name");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_already_snake_case() {
        let tool_name = ToolName::new("already_snake_case");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("already_snake_case");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_uppercase_letters() {
        let tool_name = ToolName::new("UPPERCASE_TOOL_NAME");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("uppercase_tool_name");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_numbers_only() {
        let tool_name = ToolName::new("123456");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("123456");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_mixed_numbers_and_letters() {
        let tool_name = ToolName::new("tool1Name2Test3");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("tool1name2test3");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_empty_string() {
        let tool_name = ToolName::new("");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_only_special_chars() {
        let tool_name = ToolName::new("@#$%^&*()");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_complex_mixed_case() {
        let tool_name = ToolName::new("XMLHttpRequest2Handler");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("xmlhttprequest2handler");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_dots_and_slashes() {
        let tool_name = ToolName::new("tool.name/with.dots");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("tool_name_with_dots");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_single_underscore_preserved() {
        let tool_name = ToolName::new("tool_name");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("tool_name");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_camel_case_with_underscore() {
        let tool_name = ToolName::new("camelCase_withUnderscore");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("camelcase_withunderscore");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_numbers_between_letters() {
        let tool_name = ToolName::new("tool1tool2tool3");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("tool1tool2tool3");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sanitize_mixed_case_preserves_numbers() {
        let tool_name = ToolName::new("Test123Case");
        let actual = tool_name.into_sanitized();
        let expected = ToolName::new("test123case");
        assert_eq!(actual, expected);
    }
}
