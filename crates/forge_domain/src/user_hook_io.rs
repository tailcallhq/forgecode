use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Exit code constants for hook script results.
pub mod exit_codes {
    /// Hook executed successfully. stdout may contain JSON output.
    pub const SUCCESS: i32 = 0;
    /// Blocking error. stderr is used as feedback message.
    pub const BLOCK: i32 = 2;
}

/// JSON input sent to hook scripts via stdin.
///
/// Contains common fields shared across all hook events plus event-specific
/// data in the `event_data` field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookInput {
    /// The hook event name (e.g., "PreToolUse", "PostToolUse", "Stop").
    pub hook_event_name: String,

    /// Current working directory.
    pub cwd: String,

    /// Session/conversation ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// Event-specific payload data.
    #[serde(flatten)]
    pub event_data: HookEventInput,
}

/// Event-specific input data variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum HookEventInput {
    /// Input for PreToolUse events.
    PreToolUse {
        /// Name of the tool being called.
        tool_name: String,
        /// Tool call arguments as a JSON value.
        tool_input: Value,
        /// Unique identifier for this tool call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
    },
    /// Input for PostToolUse events.
    PostToolUse {
        /// Name of the tool that was called.
        tool_name: String,
        /// Tool call arguments as a JSON value.
        tool_input: Value,
        /// Tool output/response as a JSON value.
        tool_response: Value,
        /// Unique identifier for this tool call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
    },
    /// Input for Stop events.
    Stop {
        /// Whether a previous Stop hook caused this continuation. Hook scripts
        /// should check this to prevent infinite loops.
        #[serde(default)]
        stop_hook_active: bool,
        /// The last assistant message text before the stop event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_assistant_message: Option<String>,
    },
    /// Input for SessionStart events.
    SessionStart {
        /// Source of the session start (e.g., "startup", "resume").
        source: String,
    },
    /// Input for UserPromptSubmit events.
    UserPromptSubmit {
        /// The raw prompt text submitted by the user.
        prompt: String,
    },
    /// Empty input for events that don't need event-specific data.
    Empty {},
}

/// JSON output parsed from hook script stdout.
///
/// Fields are optional; scripts that don't need to control behavior can simply
/// exit 0 with empty stdout.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookOutput {
    /// Whether execution should continue. When `false`, prevents the agent's
    /// execution loop from continuing. Checked by `is_blocking()` alongside
    /// `decision` and `permission_decision`.
    #[serde(default, rename = "continue", skip_serializing_if = "Option::is_none")]
    pub continue_execution: Option<bool>,

    /// Decision for blocking events. `"block"` blocks the operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,

    /// Reason for blocking, used as feedback to the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// For PreToolUse: permission decision ("allow", "deny", "ask").
    #[serde(
        default,
        rename = "permissionDecision",
        skip_serializing_if = "Option::is_none"
    )]
    pub permission_decision: Option<String>,

    /// For PreToolUse: modified tool input to replace the original.
    #[serde(
        default,
        rename = "updatedInput",
        skip_serializing_if = "Option::is_none"
    )]
    pub updated_input: Option<Map<String, Value>>,

    /// Additional context to inject into the conversation.
    #[serde(
        default,
        rename = "additionalContext",
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_context: Option<String>,

    /// Reason for stopping, used as a fallback reason when
    /// `continue_execution` is `false`. Consumed by `process_results` and
    /// `process_pre_tool_use_output` as a fallback when `reason` is absent.
    #[serde(
        default,
        rename = "stopReason",
        skip_serializing_if = "Option::is_none"
    )]
    pub stop_reason: Option<String>,
}

impl HookOutput {
    /// Attempts to parse stdout as JSON. Falls back to empty output on failure.
    pub fn parse(stdout: &str) -> Self {
        if stdout.trim().is_empty() {
            return Self::default();
        }
        serde_json::from_str(stdout).unwrap_or_default()
    }

    /// Returns true if this output requests blocking.
    pub fn is_blocking(&self) -> bool {
        self.decision.as_deref() == Some("block")
            || self.permission_decision.as_deref() == Some("deny")
            || self.continue_execution == Some(false)
    }

    /// Returns the blocking reason, preferring `reason` over `stop_reason`.
    pub fn blocking_reason(&self, default: &str) -> String {
        self.reason
            .clone()
            .or_else(|| self.stop_reason.clone())
            .unwrap_or_else(|| default.to_string())
    }
}

/// Result of executing a hook command.
#[derive(Debug, Clone)]
pub struct HookExecutionResult {
    /// Process exit code (None if terminated by signal).
    pub exit_code: Option<i32>,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
}

impl HookExecutionResult {
    /// Returns true if the hook exited with the blocking exit code (2).
    pub fn is_blocking_exit(&self) -> bool {
        self.exit_code == Some(exit_codes::BLOCK)
    }

    /// Returns true if the hook exited successfully (0).
    pub fn is_success(&self) -> bool {
        self.exit_code == Some(exit_codes::SUCCESS)
    }

    /// Returns true if the hook exited with a non-blocking error (non-0,
    /// non-2).
    pub fn is_non_blocking_error(&self) -> bool {
        match self.exit_code {
            Some(code) => code != exit_codes::SUCCESS && code != exit_codes::BLOCK,
            None => true,
        }
    }

    /// Parses the stdout as a HookOutput if the exit was successful.
    pub fn parse_output(&self) -> Option<HookOutput> {
        if self.is_success() {
            Some(HookOutput::parse(&self.stdout))
        } else {
            None
        }
    }

    /// Returns the feedback message for blocking errors (stderr content).
    pub fn blocking_message(&self) -> Option<&str> {
        if self.is_blocking_exit() {
            let msg = self.stderr.trim();
            if msg.is_empty() { None } else { Some(msg) }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_hook_input_serialization_pre_tool_use() {
        let fixture = HookInput {
            hook_event_name: "PreToolUse".to_string(),
            cwd: "/project".to_string(),
            session_id: Some("sess-123".to_string()),
            event_data: HookEventInput::PreToolUse {
                tool_name: "Bash".to_string(),
                tool_input: serde_json::json!({"command": "ls"}),
                tool_use_id: None,
            },
        };

        let actual = serde_json::to_value(&fixture).unwrap();

        assert_eq!(actual["hook_event_name"], "PreToolUse");
        assert_eq!(actual["cwd"], "/project");
        assert_eq!(actual["tool_name"], "Bash");
        assert_eq!(actual["tool_input"]["command"], "ls");
        assert!(actual.get("tool_use_id").is_none());
    }

    #[test]
    fn test_hook_input_serialization_pre_tool_use_with_tool_use_id() {
        let fixture = HookInput {
            hook_event_name: "PreToolUse".to_string(),
            cwd: "/project".to_string(),
            session_id: Some("sess-123".to_string()),
            event_data: HookEventInput::PreToolUse {
                tool_name: "Bash".to_string(),
                tool_input: serde_json::json!({"command": "ls"}),
                tool_use_id: Some("forge_call_id_abc123".to_string()),
            },
        };

        let actual = serde_json::to_value(&fixture).unwrap();

        assert_eq!(actual["tool_use_id"], "forge_call_id_abc123");
    }

    #[test]
    fn test_hook_input_serialization_stop() {
        let fixture = HookInput {
            hook_event_name: "Stop".to_string(),
            cwd: "/project".to_string(),
            session_id: None,
            event_data: HookEventInput::Stop {
                stop_hook_active: false,
                last_assistant_message: None,
            },
        };

        let actual = serde_json::to_value(&fixture).unwrap();

        assert_eq!(actual["hook_event_name"], "Stop");
        assert!(actual.get("last_assistant_message").is_none());
    }

    #[test]
    fn test_hook_input_serialization_stop_with_last_assistant_message() {
        let fixture = HookInput {
            hook_event_name: "Stop".to_string(),
            cwd: "/project".to_string(),
            session_id: Some("sess-456".to_string()),
            event_data: HookEventInput::Stop {
                stop_hook_active: false,
                last_assistant_message: Some("Here is the result.".to_string()),
            },
        };

        let actual = serde_json::to_value(&fixture).unwrap();

        assert_eq!(actual["last_assistant_message"], "Here is the result.");
    }

    #[test]
    fn test_hook_input_serialization_user_prompt_submit() {
        let fixture = HookInput {
            hook_event_name: "UserPromptSubmit".to_string(),
            cwd: "/project".to_string(),
            session_id: Some("sess-abc".to_string()),
            event_data: HookEventInput::UserPromptSubmit { prompt: "fix the bug".to_string() },
        };

        let actual = serde_json::to_value(&fixture).unwrap();

        assert_eq!(actual["hook_event_name"], "UserPromptSubmit");
        assert_eq!(actual["cwd"], "/project");
        assert_eq!(actual["session_id"], "sess-abc");
        assert_eq!(actual["prompt"], "fix the bug");
        // No tool_name or other variant fields present
        assert!(actual["tool_name"].is_null());
    }

    #[test]
    fn test_hook_output_parse_valid_json() {
        let stdout = r#"{"decision": "block", "reason": "unsafe command"}"#;
        let actual = HookOutput::parse(stdout);

        assert_eq!(actual.decision, Some("block".to_string()));
        assert_eq!(actual.reason, Some("unsafe command".to_string()));
    }

    #[test]
    fn test_hook_output_parse_empty_string() {
        let actual = HookOutput::parse("");
        let expected = HookOutput::default();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_hook_output_parse_invalid_json_returns_default() {
        let actual = HookOutput::parse("not json at all");
        let expected = HookOutput::default();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_hook_output_is_blocking() {
        let fixture = HookOutput { decision: Some("block".to_string()), ..Default::default() };
        assert!(fixture.is_blocking());

        let fixture = HookOutput {
            permission_decision: Some("deny".to_string()),
            ..Default::default()
        };
        assert!(fixture.is_blocking());

        let fixture = HookOutput::default();
        assert!(!fixture.is_blocking());
    }

    #[test]
    fn test_hook_output_is_blocking_continue_false() {
        let fixture = HookOutput { continue_execution: Some(false), ..Default::default() };
        assert!(fixture.is_blocking());
    }

    #[test]
    fn test_hook_output_is_not_blocking_continue_true() {
        let fixture = HookOutput { continue_execution: Some(true), ..Default::default() };
        assert!(!fixture.is_blocking());
    }

    #[test]
    fn test_hook_output_is_not_blocking_continue_none() {
        let fixture = HookOutput { continue_execution: None, ..Default::default() };
        assert!(!fixture.is_blocking());
    }

    #[test]
    fn test_hook_output_continue_false_with_stop_reason_parses_and_blocks() {
        let stdout = r#"{"continue": false, "stopReason": "done"}"#;
        let actual = HookOutput::parse(stdout);
        assert!(actual.is_blocking());
        assert_eq!(actual.continue_execution, Some(false));
        assert_eq!(actual.stop_reason, Some("done".to_string()));
    }

    #[test]
    fn test_blocking_reason_prefers_reason_over_stop_reason() {
        let fixture = HookOutput {
            reason: Some("primary".to_string()),
            stop_reason: Some("secondary".to_string()),
            ..Default::default()
        };
        let actual = fixture.blocking_reason("default");
        assert_eq!(actual, "primary");
    }

    #[test]
    fn test_blocking_reason_falls_back_to_stop_reason() {
        let fixture = HookOutput {
            stop_reason: Some("fallback".to_string()),
            ..Default::default()
        };
        let actual = fixture.blocking_reason("default");
        assert_eq!(actual, "fallback");
    }

    #[test]
    fn test_blocking_reason_uses_default_when_both_none() {
        let fixture = HookOutput::default();
        let actual = fixture.blocking_reason("default reason");
        assert_eq!(actual, "default reason");
    }

    #[test]
    fn test_hook_execution_result_blocking() {
        let fixture = HookExecutionResult {
            exit_code: Some(2),
            stdout: String::new(),
            stderr: "Blocked: unsafe command".to_string(),
        };

        assert!(fixture.is_blocking_exit());
        assert!(!fixture.is_success());
        assert!(!fixture.is_non_blocking_error());
        assert_eq!(fixture.blocking_message(), Some("Blocked: unsafe command"));
        assert!(fixture.parse_output().is_none());
    }

    #[test]
    fn test_hook_execution_result_success() {
        let fixture = HookExecutionResult {
            exit_code: Some(0),
            stdout: r#"{"decision": "block", "reason": "test"}"#.to_string(),
            stderr: String::new(),
        };

        assert!(fixture.is_success());
        assert!(!fixture.is_blocking_exit());
        assert!(!fixture.is_non_blocking_error());
        let output = fixture.parse_output().unwrap();
        assert!(output.is_blocking());
    }

    #[test]
    fn test_hook_execution_result_non_blocking_error() {
        let fixture = HookExecutionResult {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "some error".to_string(),
        };

        assert!(fixture.is_non_blocking_error());
        assert!(!fixture.is_success());
        assert!(!fixture.is_blocking_exit());
        assert!(fixture.blocking_message().is_none());
    }

    // --- Schema validation tests for updatedInput ---

    #[test]
    fn test_updated_input_valid_object_parsed() {
        let stdout = r#"{"updatedInput": {"command": "echo safe"}}"#;
        let actual = HookOutput::parse(stdout);
        let expected_map = Map::from_iter([(
            "command".to_string(),
            Value::String("echo safe".to_string()),
        )]);
        assert_eq!(actual.updated_input, Some(expected_map));
    }

    #[test]
    fn test_updated_input_string_rejected_falls_back_to_default() {
        // updatedInput is a string, not an object => serde rejects it,
        // entire parse falls back to default (updated_input = None).
        let stdout = r#"{"updatedInput": "not an object"}"#;
        let actual = HookOutput::parse(stdout);
        assert_eq!(actual.updated_input, None);
    }

    #[test]
    fn test_updated_input_number_rejected_falls_back_to_default() {
        let stdout = r#"{"updatedInput": 42}"#;
        let actual = HookOutput::parse(stdout);
        assert_eq!(actual.updated_input, None);
    }

    #[test]
    fn test_updated_input_array_rejected_falls_back_to_default() {
        let stdout = r#"{"updatedInput": [1, 2, 3]}"#;
        let actual = HookOutput::parse(stdout);
        assert_eq!(actual.updated_input, None);
    }

    #[test]
    fn test_updated_input_bool_rejected_falls_back_to_default() {
        let stdout = r#"{"updatedInput": true}"#;
        let actual = HookOutput::parse(stdout);
        assert_eq!(actual.updated_input, None);
    }

    #[test]
    fn test_updated_input_null_treated_as_none() {
        // JSON null for an Option<Map> field => None (not Some(empty map))
        let stdout = r#"{"updatedInput": null}"#;
        let actual = HookOutput::parse(stdout);
        assert_eq!(actual.updated_input, None);
    }

    #[test]
    fn test_updated_input_nested_object_accepted() {
        let stdout = r#"{"updatedInput": {"a": {"b": [1, 2]}, "c": true}}"#;
        let actual = HookOutput::parse(stdout);
        let expected_map = Map::from_iter([
            ("a".to_string(), serde_json::json!({"b": [1, 2]})),
            ("c".to_string(), Value::Bool(true)),
        ]);
        assert_eq!(actual.updated_input, Some(expected_map));
    }

    #[test]
    fn test_malformed_updated_input_preserves_other_fields() {
        // When updatedInput is invalid, the entire HookOutput parse fails
        // and falls back to default. This means other fields like `decision`
        // are also lost. This is the expected behavior — a malformed hook
        // output is treated as if the hook returned nothing.
        let stdout = r#"{"decision": "block", "updatedInput": "bad"}"#;
        let actual = HookOutput::parse(stdout);
        assert_eq!(actual, HookOutput::default());
    }
}
