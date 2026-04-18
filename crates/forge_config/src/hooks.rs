use std::collections::HashMap;

use fake::Dummy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::Display;

/// Top-level user hook configuration.
///
/// Maps hook event names to a list of matcher groups. This is deserialized
/// from the `hooks` section in `.forge.toml`.
///
/// Example TOML:
/// ```toml
/// [[hooks.PreToolUse]]
/// matcher = "Bash"
///
///   [[hooks.PreToolUse.hooks]]
///   type = "command"
///   command = "echo hi"
/// ```
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, Dummy)]
pub struct UserHookConfig {
    /// Map of event name -> list of matcher groups.
    #[serde(flatten)]
    pub events: HashMap<UserHookEventName, Vec<UserHookMatcherGroup>>,
}

impl UserHookConfig {
    /// Creates an empty user hook configuration.
    pub fn new() -> Self {
        Self { events: HashMap::new() }
    }

    /// Returns the matcher groups for a given event name, or an empty slice if
    /// none.
    pub fn get_groups(&self, event: &UserHookEventName) -> &[UserHookMatcherGroup] {
        self.events.get(event).map_or(&[], |v| v.as_slice())
    }

    /// Returns true if no hook events are configured.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Supported hook event names that map to lifecycle points in the
/// orchestrator.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display, JsonSchema, Dummy)]
pub enum UserHookEventName {
    /// Fired before a tool call executes. Can block execution.
    PreToolUse,
    /// Fired after a tool call succeeds.
    PostToolUse,
    /// Fired after a tool call fails.
    PostToolUseFailure,
    /// Fired when the agent finishes responding. Can block stop to continue.
    Stop,
    /// Fired when a session starts or resumes.
    SessionStart,
    /// Fired when a session ends/terminates.
    SessionEnd,
    /// Fired when a user prompt is submitted.
    UserPromptSubmit,
}

/// A matcher group pairs an optional regex matcher with a list of hook
/// handlers.
///
/// When a lifecycle event fires, only matcher groups whose `matcher` regex
/// matches the relevant event context (e.g., tool name) will have their hooks
/// executed. If `matcher` is `None` (or an empty string, which is normalized
/// to `None`), all hooks in this group fire unconditionally.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, Dummy)]
pub struct UserHookMatcherGroup {
    /// Optional regex pattern to match against (e.g., tool name for
    /// PreToolUse/PostToolUse).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,

    /// List of hook handlers to execute when this matcher matches.
    #[serde(default)]
    pub hooks: Vec<UserHookEntry>,
}

/// A single hook handler entry that defines what action to take.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, Dummy)]
pub struct UserHookEntry {
    /// The type of hook handler.
    #[serde(rename = "type")]
    pub hook_type: UserHookType,

    /// The shell command to execute (for `Command` type hooks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// Timeout in milliseconds for this hook. Defaults to 600000ms (10
    /// minutes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

/// The type of hook handler to execute.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, Dummy)]
#[serde(rename_all = "lowercase")]
pub enum UserHookType {
    /// Executes a shell command, piping JSON to stdin and reading JSON from
    /// stdout.
    Command,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_deserialize_empty_config() {
        let toml = "";
        let actual: UserHookConfig = toml_edit::de::from_str(toml).unwrap();
        let expected = UserHookConfig::new();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_deserialize_pre_tool_use_hook() {
        let toml = include_str!("fixtures/hook_pre_tool_use.toml");
        let actual: UserHookConfig = toml_edit::de::from_str(toml).unwrap();
        let groups = actual.get_groups(&UserHookEventName::PreToolUse);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].matcher, Some("Bash".to_string()));
        assert_eq!(groups[0].hooks.len(), 1);
        assert_eq!(groups[0].hooks[0].hook_type, UserHookType::Command);
        assert_eq!(
            groups[0].hooks[0].command,
            Some("echo 'blocked'".to_string())
        );
    }

    #[test]
    fn test_deserialize_multiple_events() {
        let toml = include_str!("fixtures/hook_multiple_events.toml");
        let actual: UserHookConfig = toml_edit::de::from_str(toml).unwrap();

        assert_eq!(actual.get_groups(&UserHookEventName::PreToolUse).len(), 1);
        assert_eq!(actual.get_groups(&UserHookEventName::PostToolUse).len(), 1);
        assert_eq!(actual.get_groups(&UserHookEventName::Stop).len(), 1);
        assert!(
            actual
                .get_groups(&UserHookEventName::SessionStart)
                .is_empty()
        );
    }

    #[test]
    fn test_deserialize_hook_with_timeout() {
        let toml = include_str!("fixtures/hook_with_timeout.toml");
        let actual: UserHookConfig = toml_edit::de::from_str(toml).unwrap();
        let groups = actual.get_groups(&UserHookEventName::PreToolUse);

        assert_eq!(groups[0].hooks[0].timeout, Some(30000));
    }

    #[test]
    fn test_no_matcher_group_fires_unconditionally() {
        let toml = include_str!("fixtures/hook_no_matcher.toml");
        let actual: UserHookConfig = toml_edit::de::from_str(toml).unwrap();
        let groups = actual.get_groups(&UserHookEventName::PostToolUse);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].matcher, None);
    }

    #[test]
    fn test_toml_round_trip() {
        let toml_input = r#"
[[PreToolUse]]
matcher = "Bash"

  [[PreToolUse.hooks]]
  type = "command"
  command = "check.sh"
  timeout = 5000
"#;
        let config: UserHookConfig = toml_edit::de::from_str(toml_input).unwrap();
        let serialized = toml_edit::ser::to_string_pretty(&config).unwrap();
        let roundtrip: UserHookConfig = toml_edit::de::from_str(&serialized).unwrap();
        assert_eq!(config, roundtrip);
    }

    #[test]
    fn test_json_deserialization_still_works() {
        let json = r#"{
            "PreToolUse": [
                { "matcher": "Bash", "hooks": [{ "type": "command", "command": "echo hi" }] }
            ]
        }"#;
        let actual: UserHookConfig = serde_json::from_str(json).unwrap();
        assert_eq!(actual.get_groups(&UserHookEventName::PreToolUse).len(), 1);
    }
}
