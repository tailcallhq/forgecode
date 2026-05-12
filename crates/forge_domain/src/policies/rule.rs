use std::fmt::{Display, Formatter};
use std::path::Path;

use glob::Pattern;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::operation::PermissionOperation;

/// Rule for write operations with a glob pattern
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct WriteRule {
    pub write: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// Rule for read operations with a glob pattern
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct ReadRule {
    pub read: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// Rule for execute operations with a command pattern
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteRule {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// Rule for network fetch operations with a URL pattern
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct Fetch {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// Rule for MCP server connection authorization matched by server-name glob.
///
/// MCP rules are intentionally scope-free: trust for a server is per-config
/// (project's `.mcp.json` vs. the user-level one), and that distinction is
/// resolved at the service layer before the policy engine ever runs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct McpRule {
    /// Glob over the MCP server name as it appears in `.mcp.json`.
    pub mcp: String,
}

/// Rules that define what operations are covered by a policy
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Rule {
    /// Rule for write operations with a glob pattern
    Write(WriteRule),
    /// Rule for read operations with a glob pattern
    Read(ReadRule),
    /// Rule for execute operations with a command pattern
    Execute(ExecuteRule),
    /// Rule for network fetch operations with a URL pattern
    Fetch(Fetch),
    /// Rule for MCP tool invocations with a tool-name glob pattern
    Mcp(McpRule),
}

impl Rule {
    /// Check if this rule matches the given operation
    pub fn matches(&self, operation: &PermissionOperation) -> bool {
        match (self, operation) {
            (Rule::Write(rule), PermissionOperation::Write { path, cwd, message: _ }) => {
                let pattern_matches = match_pattern(&rule.write, path);
                let dir = match &rule.dir {
                    Some(wd_pattern) => match_pattern(wd_pattern, cwd),
                    None => true, /* If no working directory pattern is specified, it matches any
                                   * directory */
                };
                pattern_matches && dir
            }
            (Rule::Read(rule), PermissionOperation::Read { path, cwd, message: _ }) => {
                let pattern_matches = match_pattern(&rule.read, path);
                let dir_matches = match &rule.dir {
                    Some(wd_pattern) => match_pattern(wd_pattern, cwd),
                    None => true, /* If no working directory pattern is specified, it matches any
                                   * directory */
                };
                pattern_matches && dir_matches
            }

            (Rule::Execute(rule), PermissionOperation::Execute { command: cmd, cwd }) => {
                let command_matches = match_pattern(&rule.command, cmd);
                let dir_matches = match &rule.dir {
                    Some(wd_pattern) => match_pattern(wd_pattern, cwd),
                    None => true, /* If no working directory pattern is specified, it matches any
                                   * directory */
                };
                command_matches && dir_matches
            }
            (Rule::Fetch(rule), PermissionOperation::Fetch { url, cwd, message: _ }) => {
                let url_matches = match_pattern(&rule.url, url);
                let dir_matches = match &rule.dir {
                    Some(wd_pattern) => match_pattern(wd_pattern, cwd),
                    None => true, /* If no working directory pattern is specified, it matches any
                                   * directory */
                };
                url_matches && dir_matches
            }
            (Rule::Mcp(rule), PermissionOperation::Mcp { server, message: _ }) => {
                match_pattern(&rule.mcp, server)
            }
            _ => false,
        }
    }
}

/// Helper function to match a glob pattern against a path or string
fn match_pattern<P: AsRef<Path>>(pattern: &str, target: P) -> bool {
    match Pattern::new(pattern) {
        Ok(glob_pattern) => {
            let target_str = target.as_ref().to_string_lossy();
            glob_pattern.matches(&target_str)
        }
        Err(_) => false, // Invalid pattern doesn't match anything
    }
}

impl Display for WriteRule {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some(wd) = &self.dir {
            write!(f, "write '{}' in '{}'", self.write, wd)
        } else {
            write!(f, "write '{}'", self.write)
        }
    }
}

impl Display for ReadRule {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some(wd) = &self.dir {
            write!(f, "read '{}' in '{}'", self.read, wd)
        } else {
            write!(f, "read '{}'", self.read)
        }
    }
}

impl Display for ExecuteRule {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some(wd) = &self.dir {
            write!(f, "execute '{}' in '{}'", self.command, wd)
        } else {
            write!(f, "execute '{}'", self.command)
        }
    }
}

impl Display for Fetch {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some(wd) = &self.dir {
            write!(f, "fetch '{}' in '{}'", self.url, wd)
        } else {
            write!(f, "fetch '{}'", self.url)
        }
    }
}

impl Display for McpRule {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "mcp server '{}'", self.mcp)
    }
}

impl Display for Rule {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Rule::Write(rule) => write!(f, "{rule}"),
            Rule::Read(rule) => write!(f, "{rule}"),
            Rule::Execute(rule) => write!(f, "{rule}"),
            Rule::Fetch(rule) => write!(f, "{rule}"),
            Rule::Mcp(rule) => write!(f, "{rule}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::*;

    fn fixture_write_operation() -> PermissionOperation {
        PermissionOperation::Write {
            path: PathBuf::from("src/main.rs"),
            cwd: PathBuf::from("/home/user/project"),
            message: "Create/overwrite file: src/main.rs".to_string(),
        }
    }

    fn fixture_patch_operation() -> PermissionOperation {
        PermissionOperation::Write {
            path: PathBuf::from("src/main.rs"),
            cwd: PathBuf::from("/home/user/project"),
            message: "Modify file: src/main.rs".to_string(),
        }
    }

    fn fixture_read_operation() -> PermissionOperation {
        PermissionOperation::Read {
            path: PathBuf::from("config/dev.yml"),
            cwd: PathBuf::from("/home/user/project"),
            message: "Read file: config/dev.yml".to_string(),
        }
    }

    fn fixture_execute_operation() -> PermissionOperation {
        PermissionOperation::Execute {
            command: "cargo build".to_string(),
            cwd: PathBuf::from("/home/user/project"),
        }
    }

    fn fixture_net_fetch_operation() -> PermissionOperation {
        PermissionOperation::Fetch {
            url: "https://api.example.com/data".to_string(),
            cwd: PathBuf::from("/home/user/project"),
            message: "Fetch content from URL: https://api.example.com/data".to_string(),
        }
    }

    fn fixture_mcp_operation() -> PermissionOperation {
        PermissionOperation::Mcp {
            server: "github".to_string(),
            message: "Execute MCP tool: mcp_github_tool_create_issue".to_string(),
        }
    }

    #[test]
    fn test_rule_matches_write_operation() {
        let fixture = Rule::Write(WriteRule { write: "src/**/*.rs".to_string(), dir: None });
        let operation = fixture_write_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_rule_matches_write_operation_with_patch_scenario() {
        let fixture = Rule::Write(WriteRule { write: "src/**/*.rs".to_string(), dir: None });
        let operation = fixture_patch_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_rule_does_not_match_different_operation() {
        let fixture = Rule::Read(ReadRule { read: "config/*.yml".to_string(), dir: None });
        let operation = fixture_write_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }

    #[test]
    fn test_match_pattern_exact_match() {
        let actual = match_pattern("src/main.rs", "src/main.rs");

        assert_eq!(actual, true);
    }

    #[test]
    fn test_match_pattern_glob_wildcard() {
        let actual = match_pattern("src/**/*.rs", "src/lib/main.rs");

        assert_eq!(actual, true);
    }

    #[test]
    fn test_match_pattern_no_match() {
        let actual = match_pattern("src/**/*.rs", "docs/readme.md");

        assert_eq!(actual, false);
    }

    #[test]
    fn test_execute_command_pattern_match() {
        let fixture = Rule::Execute(ExecuteRule { command: "cargo *".to_string(), dir: None });
        let operation = fixture_execute_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_read_config_pattern_match() {
        let fixture = Rule::Read(ReadRule { read: "config/*.yml".to_string(), dir: None });
        let operation = fixture_read_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_net_fetch_url_pattern_match() {
        let fixture =
            Rule::Fetch(Fetch { url: "https://api.example.com/*".to_string(), dir: None });
        let operation = fixture_net_fetch_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_execute_working_directory_pattern_match() {
        let fixture = Rule::Execute(ExecuteRule {
            command: "cargo *".to_string(),
            dir: Some("/home/user/*".to_string()),
        });
        let operation = fixture_execute_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_execute_working_directory_pattern_no_match() {
        let fixture = Rule::Execute(ExecuteRule {
            command: "cargo *".to_string(),
            dir: Some("/different/path/*".to_string()),
        });
        let operation = fixture_execute_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }

    #[test]
    fn test_execute_no_working_directory_pattern_matches_any() {
        let fixture = Rule::Execute(ExecuteRule { command: "cargo *".to_string(), dir: None });
        let operation = fixture_execute_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_rule_exact_match() {
        let fixture = Rule::Mcp(McpRule { mcp: "github".to_string() });
        let operation = fixture_mcp_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_rule_glob_wildcard() {
        let fixture = Rule::Mcp(McpRule { mcp: "git*".to_string() });
        let operation = fixture_mcp_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_rule_does_not_match_other_server() {
        let fixture = Rule::Mcp(McpRule { mcp: "slack".to_string() });
        let operation = fixture_mcp_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }

    #[test]
    fn test_mcp_rule_does_not_match_non_mcp_operation() {
        let fixture = Rule::Mcp(McpRule { mcp: "*".to_string() });
        let operation = fixture_execute_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }
}
