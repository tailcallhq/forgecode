use std::fmt::{Display, Formatter};
use std::path::Path;

use glob::Pattern;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::operation::PermissionOperation;
use crate::mcp::McpServerConfig;

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

/// Filter criteria nested inside an [`McpRule`]. All fields are optional;
/// omitting a field means "match any value" for that dimension. Multiple
/// fields are combined with logical AND.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct McpFilter {
    /// Optional glob over the command used to launch a stdio MCP server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Optional glob patterns over the server's argument list (all must match).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Optional glob over the URL of an HTTP/SSE MCP server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Optional working directory glob pattern. `None` matches any directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// Rule for MCP server connection authorization. The required `mcp` key
/// identifies this as an MCP rule (analogous to `write:`, `read:`, etc.) and
/// disambiguates it from other rule types in the untagged `Rule` enum.
///
/// The value is an [`McpFilter`] object whose fields are all optional:
/// an empty object `{}` matches any MCP server; populating fields narrows the
/// match. Stdio servers are matched via `command`/`args`; HTTP servers via
/// `url`. Specifying both `command` and `url` will never match (a server is
/// either stdio or HTTP, not both).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct McpRule {
    /// Filter criteria for the MCP server. Use `{}` to match any server.
    pub mcp: McpFilter,
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
            (Rule::Write(rule), PermissionOperation::Write { path, cwd, .. }) => {
                match_pattern(&rule.write, path) && match_dir(&rule.dir, cwd)
            }
            (Rule::Read(rule), PermissionOperation::Read { path, cwd, .. }) => {
                match_pattern(&rule.read, path) && match_dir(&rule.dir, cwd)
            }
            (Rule::Execute(rule), PermissionOperation::Execute { command: cmd, cwd }) => {
                match_pattern(&rule.command, cmd) && match_dir(&rule.dir, cwd)
            }
            (Rule::Fetch(rule), PermissionOperation::Fetch { url, cwd, .. }) => {
                match_pattern(&rule.url, url) && match_dir(&rule.dir, cwd)
            }
            (Rule::Mcp(rule), PermissionOperation::Mcp { config, cwd, .. }) => {
                rule.mcp.matches_config(config) && match_dir(&rule.mcp.dir, cwd)
            }
            _ => false,
        }
    }
}

/// Returns true when `opt_pattern` is absent (wildcard) or matches `target`.
fn match_dir<P: AsRef<Path>>(opt_pattern: &Option<String>, target: P) -> bool {
    opt_pattern
        .as_deref()
        .is_none_or(|pat| match_pattern(pat, target))
}

/// Returns true when `pattern` glob-matches `target`.
fn match_pattern<P: AsRef<Path>>(pattern: &str, target: P) -> bool {
    Pattern::new(pattern).is_ok_and(|p| p.matches(&target.as_ref().to_string_lossy()))
}

impl McpFilter {
    /// Build a filter that exactly pins `config` — stdio servers are matched by
    /// `command` + `args`; HTTP servers by `url`. The `dir` is always set to
    /// `cwd` so the rule is scoped to the working directory.
    pub fn from_config(config: &McpServerConfig, cwd: &std::path::Path) -> Self {
        let dir = Some(cwd.to_string_lossy().into_owned());
        match config {
            McpServerConfig::Stdio(s) => Self {
                command: Some(s.command.clone()),
                args: if s.args.is_empty() {
                    None
                } else {
                    Some(s.args.clone())
                },
                url: None,
                dir,
            },
            McpServerConfig::Http(h) => {
                Self { command: None, args: None, url: Some(h.url.clone()), dir }
            }
        }
    }

    /// Returns true when this filter is compatible with `config`.
    ///
    /// A stdio filter (has `command`/`args`, no `url`) only matches stdio
    /// servers; an HTTP filter (has `url`, no `command`/`args`) only
    /// matches HTTP servers; an empty filter matches both.
    fn matches_config(&self, config: &McpServerConfig) -> bool {
        match config {
            McpServerConfig::Stdio(s) => {
                // A url-only rule must not match a stdio server
                self.url.is_none()
                    && self
                        .command
                        .as_deref()
                        .is_none_or(|p| match_pattern(p, &s.command))
                    && self.args.as_deref().is_none_or(|patterns| {
                        patterns
                            .iter()
                            .all(|p| s.args.iter().any(|a| match_pattern(p, a)))
                    })
            }
            McpServerConfig::Http(h) => {
                // A command/args-only rule must not match an HTTP server
                self.command.is_none()
                    && self.args.is_none()
                    && self.url.as_deref().is_none_or(|p| match_pattern(p, &h.url))
            }
        }
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
        let filter = &self.mcp;
        let mut parts: Vec<String> = Vec::new();
        if let Some(cmd) = &filter.command {
            parts.push(format!("command '{cmd}'"));
        }
        if let Some(args) = &filter.args {
            parts.push(format!("args [{}]", args.join(", ")));
        }
        if let Some(url) = &filter.url {
            parts.push(format!("url '{url}'"));
        }
        let base = if parts.is_empty() {
            "mcp server (any)".to_string()
        } else {
            format!("mcp server with {}", parts.join(", "))
        };
        if let Some(wd) = &filter.dir {
            write!(f, "{} in '{wd}'", base)
        } else {
            write!(f, "{}", base)
        }
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
    use crate::mcp::McpServerConfig;

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

    fn fixture_mcp_stdio_operation() -> PermissionOperation {
        PermissionOperation::Mcp {
            config: McpServerConfig::new_stdio(
                "npx",
                vec!["-y".to_string(), "@github/mcp".to_string()],
                None,
            ),
            cwd: PathBuf::from("/home/user/project"),
            message: "Connect to MCP server: github".to_string(),
        }
    }

    fn fixture_mcp_http_operation() -> PermissionOperation {
        PermissionOperation::Mcp {
            config: McpServerConfig::new_http("https://mcp.example.com/sse"),
            cwd: PathBuf::from("/home/user/project"),
            message: "Connect to MCP server: example".to_string(),
        }
    }

    fn fixture_mcp_rule(filter: McpFilter) -> Rule {
        Rule::Mcp(McpRule { mcp: filter })
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

    // ── MCP stdio tests ──────────────────────────────────────────────────────

    #[test]
    fn test_mcp_stdio_empty_filter_matches_any_stdio() {
        let fixture = fixture_mcp_rule(McpFilter::default());
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_stdio_command_exact_match() {
        let fixture = fixture_mcp_rule(McpFilter {
            command: Some("npx".to_string()),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_stdio_command_glob_match() {
        let fixture = fixture_mcp_rule(McpFilter {
            command: Some("np*".to_string()),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_stdio_command_no_match() {
        let fixture = fixture_mcp_rule(McpFilter {
            command: Some("node".to_string()),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }

    #[test]
    fn test_mcp_stdio_args_match() {
        let fixture = fixture_mcp_rule(McpFilter {
            args: Some(vec!["@github/mcp".to_string()]),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_stdio_args_glob_match() {
        let fixture = fixture_mcp_rule(McpFilter {
            args: Some(vec!["@github/*".to_string()]),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_stdio_args_no_match() {
        let fixture = fixture_mcp_rule(McpFilter {
            args: Some(vec!["@slack/mcp".to_string()]),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }

    #[test]
    fn test_mcp_stdio_url_rule_does_not_match_stdio_server() {
        // A url-only rule must not match a stdio server
        let fixture =
            fixture_mcp_rule(McpFilter { url: Some("*".to_string()), ..McpFilter::default() });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }

    // ── MCP HTTP tests ───────────────────────────────────────────────────────

    #[test]
    fn test_mcp_http_empty_filter_matches_any_http() {
        let fixture = fixture_mcp_rule(McpFilter::default());
        let operation = fixture_mcp_http_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_http_url_exact_match() {
        let fixture = fixture_mcp_rule(McpFilter {
            url: Some("https://mcp.example.com/sse".to_string()),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_http_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_http_url_glob_match() {
        let fixture = fixture_mcp_rule(McpFilter {
            url: Some("https://mcp.example.com/*".to_string()),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_http_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_http_url_no_match() {
        let fixture = fixture_mcp_rule(McpFilter {
            url: Some("https://other.example.com/*".to_string()),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_http_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }

    #[test]
    fn test_mcp_http_command_rule_does_not_match_http_server() {
        // A command-only rule must not match an HTTP server
        let fixture =
            fixture_mcp_rule(McpFilter { command: Some("*".to_string()), ..McpFilter::default() });
        let operation = fixture_mcp_http_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }

    // ── Cross-type and dir tests ─────────────────────────────────────────────

    #[test]
    fn test_mcp_rule_does_not_match_non_mcp_operation() {
        let fixture = fixture_mcp_rule(McpFilter::default());
        let operation = fixture_execute_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }

    #[test]
    fn test_mcp_dir_pattern_matches_stdio() {
        let fixture = fixture_mcp_rule(McpFilter {
            dir: Some("/home/user/*".to_string()),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_dir_pattern_no_match_stdio() {
        let fixture = fixture_mcp_rule(McpFilter {
            dir: Some("/different/path/*".to_string()),
            ..McpFilter::default()
        });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }

    #[test]
    fn test_mcp_combined_command_and_dir_match() {
        let fixture = fixture_mcp_rule(McpFilter {
            command: Some("npx".to_string()),
            args: None,
            url: None,
            dir: Some("/home/user/*".to_string()),
        });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, true);
    }

    #[test]
    fn test_mcp_combined_command_and_dir_dir_mismatch() {
        let fixture = fixture_mcp_rule(McpFilter {
            command: Some("npx".to_string()),
            args: None,
            url: None,
            dir: Some("/different/*".to_string()),
        });
        let operation = fixture_mcp_stdio_operation();

        let actual = fixture.matches(&operation);

        assert_eq!(actual, false);
    }
}
