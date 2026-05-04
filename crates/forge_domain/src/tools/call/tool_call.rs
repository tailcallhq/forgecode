use std::collections::{HashMap, HashSet};

use derive_getters::Getters;
use derive_more::derive::From;
use derive_setters::Setters;
use serde::{Deserialize, Serialize};

use crate::xml::extract_tag_content;
use crate::{Error, Result, ToolCallArguments, ToolName, ToolResult};

/// Unique identifier for a using a tool
#[derive(Clone, Debug, Deserialize, Hash, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ToolCallId(pub(crate) String);

impl From<&str> for ToolCallId {
    fn from(value: &str) -> Self {
        ToolCallId(value.to_string())
    }
}

impl ToolCallId {
    pub fn new(value: impl ToString) -> Self {
        ToolCallId(value.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn generate() -> Self {
        let id = format!("forge_call_id_{}", uuid::Uuid::new_v4());
        ToolCallId(id)
    }
}

/// Contains a part message for using a tool. This is received as a part of the
/// response from the model only when streaming is enabled.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, Setters)]
#[setters(strip_option, into)]
pub struct ToolCallPart {
    /// Optional unique identifier that represents a single call to the tool
    /// use. NOTE: Not all models support a call ID for using a tool
    pub call_id: Option<ToolCallId>,
    pub name: Option<ToolName>,

    /// Arguments that need to be passed to the tool. NOTE: Not all tools
    /// require input
    pub arguments_part: String,

    /// Optional thought signature from Gemini3
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,

    /// Namespace for the tool call (used by OpenAI Responses API for
    /// tools discovered via tool_search with deferred loading).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, From)]
pub enum ToolCall {
    Full(ToolCallFull),
    Part(ToolCallPart),
}

impl ToolCall {
    pub fn as_partial(&self) -> Option<&ToolCallPart> {
        match self {
            ToolCall::Full(_) => None,
            ToolCall::Part(part) => Some(part),
        }
    }

    pub fn as_full(&self) -> Option<&ToolCallFull> {
        match self {
            ToolCall::Full(full) => Some(full),
            ToolCall::Part(_) => None,
        }
    }
}

/// Contains the full information about using a tool. This is received as a part
/// of the response from the model when streaming is disabled.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Setters)]
#[setters(strip_option, into)]
#[serde(rename_all = "snake_case")]
pub struct ToolCallFull {
    pub name: ToolName,
    pub call_id: Option<ToolCallId>,
    pub arguments: ToolCallArguments,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,

    /// Namespace for the tool call (used by OpenAI Responses API for
    /// tools discovered via tool_search with deferred loading).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

impl ToolCallFull {
    pub fn new(tool_name: impl Into<ToolName>) -> Self {
        Self {
            name: tool_name.into(),
            call_id: None,
            arguments: ToolCallArguments::default(),
            thought_signature: None,
            namespace: None,
        }
    }

    /// Returns true if this tool requires direct stdout/stderr access
    pub fn requires_stdout(&self) -> bool {
        crate::ToolCatalog::requires_stdout(&self.name)
    }

    pub fn try_from_parts(parts: &[ToolCallPart]) -> Result<Vec<Self>> {
        if parts.is_empty() {
            return Ok(vec![]);
        }

        let mut tool_calls = Vec::new();
        let mut current_call_id: Option<ToolCallId> = None;
        let mut current_tool_name: Option<ToolName> = None;
        let mut current_arguments = String::new();
        let mut current_thought_signature: Option<String> = None;
        let mut current_namespace: Option<String> = None;

        // GLM model workaround: Track the last valid tool name and call_id
        // GLM sends malformed tool calls where subsequent chunks have:
        // - New/different call_id
        // - Empty name
        // - Partial arguments
        // We need to associate these with the last tool call that had a valid name
        let mut last_valid_tool_name: Option<ToolName> = None;
        let mut last_valid_call_id: Option<ToolCallId> = None;

        for part in parts.iter() {
            // Check if this part has a valid tool name
            let has_valid_name = part.name.as_ref().is_some_and(|n| !n.as_str().is_empty());

            // GLM workaround: Detect GLM-style fragmented tool call
            // Pattern: empty name + non-empty args + different call_id = continuation of
            // previous tool
            let is_glm_fragment = !has_valid_name
                && !part.arguments_part.is_empty()
                && last_valid_tool_name.is_some()
                && last_valid_call_id.is_some();

            if is_glm_fragment {
                // Don't change current_call_id or current_tool_name
                // Just accumulate arguments for the existing tool
            } else if let Some(new_call_id) = &part.call_id {
                // Normal OpenAI-style handling
                if let Some(ref existing_call_id) = current_call_id
                    && existing_call_id.as_str() != new_call_id.as_str()
                {
                    // Finalize the previous tool call
                    if let Some(tool_name) = current_tool_name.take() {
                        let arguments = if current_arguments.is_empty() {
                            ToolCallArguments::default()
                        } else {
                            ToolCallArguments::from_json(current_arguments.as_str())
                        };

                        tool_calls.push(ToolCallFull {
                            name: tool_name,
                            call_id: Some(existing_call_id.clone()),
                            arguments,
                            thought_signature: current_thought_signature.take(),
                            namespace: current_namespace.take(),
                        });
                    }
                    current_arguments.clear();
                    current_thought_signature = None;
                }
                current_call_id = Some(new_call_id.clone());
            }

            if let Some(name) = &part.name
                && !name.as_str().is_empty()
            {
                current_tool_name = Some(name.clone());
                last_valid_tool_name = Some(name.clone());
                // When we get a valid name, use the current call_id as the last valid one
                if let Some(ref cid) = current_call_id {
                    last_valid_call_id = Some(cid.clone());
                }
            }

            // Capture thought_signature from the first part that has it
            if current_thought_signature.is_none() && part.thought_signature.is_some() {
                current_thought_signature = part.thought_signature.clone();
            }

            // Capture namespace from the first part that has it
            if current_namespace.is_none() && part.namespace.is_some() {
                current_namespace = part.namespace.clone();
            }

            current_arguments.push_str(&part.arguments_part);
        }

        // Finalize the last tool call
        if let Some(tool_name) = current_tool_name {
            let arguments = if current_arguments.is_empty() {
                ToolCallArguments::default()
            } else {
                ToolCallArguments::from_json(current_arguments.as_str())
            };

            tool_calls.push(ToolCallFull {
                name: tool_name,
                call_id: current_call_id,
                arguments,
                thought_signature: current_thought_signature,
                namespace: current_namespace,
            });
        }

        Ok(tool_calls)
    }

    /// Parse multiple tool calls from XML format.
    pub fn try_from_xml(input: &str) -> std::result::Result<Vec<ToolCallFull>, Error> {
        match extract_tag_content(input, "forge_tool_call") {
            None => Ok(Default::default()),
            Some(content) => {
                let mut tool_call: ToolCallFull =
                    json_repair_parse(content).map_err(|repair_error| Error::ToolCallArgument {
                        error: repair_error,
                        args: content.to_string(),
                    })?;

                // User might switch the model from a tool unsupported to tool supported model
                // leaving a lot of messages without tool calls

                tool_call.call_id = Some(ToolCallId::generate());
                Ok(vec![tool_call])
            }
        }
    }
}

fn json_repair_parse<T>(
    json_str: &str,
) -> std::result::Result<T, forge_json_repair::JsonRepairError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(json_str)
        .map_err(forge_json_repair::JsonRepairError::JsonError)
        .or_else(|_| {
            let repaired = forge_json_repair::json_repair(json_str);
            if repaired.is_ok() {
                tracing::info!("Tool call was successfully repaired.");
            }
            repaired
        })
}

#[derive(Default, Clone, Debug, Getters)]
pub struct ToolErrorTracker {
    errors: HashMap<ToolName, usize>,
    limit: usize,
}

impl ToolErrorTracker {
    pub fn new(limit: usize) -> Self {
        Self { errors: Default::default(), limit }
    }

    pub fn adjust_record(&mut self, records: &[(ToolCallFull, ToolResult)]) -> &mut Self {
        let records_iter = records.iter();
        let failed = records_iter
            .clone()
            .filter(|record| record.1.is_error())
            .map(|record| &record.1.name)
            .collect::<Vec<_>>();

        let succeeded = records_iter
            .clone()
            .filter(|record| !record.1.is_error())
            .map(|record| &record.1.name)
            .collect::<Vec<_>>();

        self.adjust(&failed, &succeeded)
    }

    pub fn failed(&mut self, tool_name: &ToolName) -> &mut Self {
        self.adjust(&[tool_name], &[])
    }

    pub fn succeed(&mut self, tool_name: &ToolName) -> &mut Self {
        self.adjust(&[], &[tool_name])
    }

    fn adjust(&mut self, failed: &[&ToolName], succeeded: &[&ToolName]) -> &mut Self {
        // Handle failures first
        let uniq_failed = failed.iter().collect::<HashSet<&&ToolName>>();
        for tool in uniq_failed.iter() {
            if let Some(count) = self.errors.get_mut(tool) {
                *count += 1;
            } else {
                self.errors.insert((**tool).to_owned(), 1);
            }
        }

        // Reset counter for tools that have clear evidence of success
        for tool in succeeded.iter().filter(|tool| !uniq_failed.contains(tool)) {
            self.errors.remove(tool);
        }

        self
    }

    fn maxed_out_tools(&self) -> Vec<&ToolName> {
        let limit = self.limit;
        self.errors
            .iter()
            .filter(|(_, count)| **count >= limit)
            .map(|data| data.0)
            .collect::<Vec<_>>()
    }

    pub fn limit_reached(&self) -> bool {
        !self.maxed_out_tools().is_empty()
    }

    pub fn error_count(&self, tool_name: &ToolName) -> usize {
        *self.errors.get(tool_name).unwrap_or(&0)
    }

    pub fn remaining_attempts(&self, tool_name: &ToolName) -> usize {
        let current_attempts = self.error_count(tool_name);
        self.limit.saturating_sub(current_attempts)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_requires_stdout_for_shell_tool() {
        let fixture = ToolCallFull::new("shell");
        assert!(fixture.requires_stdout());
    }

    #[test]
    fn test_requires_stdout_for_non_shell_tool() {
        let fixture = ToolCallFull::new("read");
        assert!(!fixture.requires_stdout());
    }

    #[test]
    fn test_multiple_calls() {
        let input = [
            ToolCallPart {
                call_id: Some(ToolCallId("call_1".to_string())),
                name: Some(ToolName::new("read")),
                arguments_part: "{\"path\": \"crates/forge_services/src/fixtures/".to_string(),
                thought_signature: None,
                namespace: None,
            },
            ToolCallPart {
                call_id: None,
                name: None,
                arguments_part: "mascot.md\"}".to_string(),
                thought_signature: None,
                namespace: None,
            },
            ToolCallPart {
                call_id: Some(ToolCallId("call_2".to_string())),
                name: Some(ToolName::new("read")),
                arguments_part: "{\"path\": \"docs/".to_string(),
                thought_signature: None,
                namespace: None,
            },
            ToolCallPart {
                // NOTE: Call ID can be repeated with each message
                call_id: Some(ToolCallId("call_2".to_string())),
                name: None,
                arguments_part: "onboarding.md\"}".to_string(),
                thought_signature: None,
                namespace: None,
            },
            ToolCallPart {
                call_id: Some(ToolCallId("call_3".to_string())),
                name: Some(ToolName::new("read")),
                arguments_part: "{\"path\": \"crates/forge_services/src/service/".to_string(),
                thought_signature: None,
                namespace: None,
            },
            ToolCallPart {
                call_id: None,
                name: None,
                arguments_part: "service.md\"}".to_string(),
                thought_signature: None,
                namespace: None,
            },
        ];

        let actual = ToolCallFull::try_from_parts(&input).unwrap();

        let expected = vec![
            ToolCallFull {
                name: ToolName::new("read"),
                call_id: Some(ToolCallId("call_1".to_string())),
                arguments: ToolCallArguments::from_json(
                    r#"{"path": "crates/forge_services/src/fixtures/mascot.md"}"#,
                ),
                thought_signature: None,
                namespace: None,
            },
            ToolCallFull {
                name: ToolName::new("read"),
                call_id: Some(ToolCallId("call_2".to_string())),
                arguments: ToolCallArguments::from_json(r#"{"path": "docs/onboarding.md"}"#),
                thought_signature: None,
                namespace: None,
            },
            ToolCallFull {
                name: ToolName::new("read"),
                call_id: Some(ToolCallId("call_3".to_string())),
                arguments: ToolCallArguments::from_json(
                    r#"{"path": "crates/forge_services/src/service/service.md"}"#,
                ),
                thought_signature: None,
                namespace: None,
            },
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_no_tools_called_returns_empty() {
        let counter = ToolErrorTracker::new(3);

        let actual = counter.maxed_out_tools();
        let expected: Vec<&ToolName> = vec![];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_only_successful_tools_never_maxed_out() {
        let read = &ToolName::new("READ");
        let write = &ToolName::new("WRITE");
        let mut counter = ToolErrorTracker::new(3);
        counter
            .adjust(&[], &[read, write])
            .adjust(&[], &[read, write, read]);

        let actual = counter.maxed_out_tools();
        let expected: Vec<&ToolName> = vec![];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_multiple_tools_maxed_out() {
        let read = &ToolName::new("READ");
        let write = &ToolName::new("WRITE");
        let mut counter = ToolErrorTracker::new(2);
        counter
            .adjust(&[read, write], &[])
            .adjust(&[read, write], &[])
            .adjust(&[read, write], &[]);

        let mut actual = counter.maxed_out_tools();
        actual.sort_by_key(|tool| tool.as_str());

        let mut expected = vec![read, write];
        expected.sort_by_key(|tool| tool.as_str());

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_tool_in_both_failed_and_succeeded_lists() {
        let read = &ToolName::new("READ");
        let mut counter = ToolErrorTracker::new(3);
        // Tool appears in both failed and succeeded - success should NOT reset due to
        // filter
        counter.adjust(&[read], &[read]);

        let actual = counter.maxed_out_tools();
        let expected: Vec<&ToolName> = vec![];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_tool_over_limit_boundary() {
        let read = &ToolName::new("READ");
        let mut counter = ToolErrorTracker::new(3);
        counter
            .adjust(&[read], &[]) // count = 1
            .adjust(&[read], &[]) // count = 2
            .adjust(&[read], &[]) // count = 3
            .adjust(&[read], &[]); // count = 4 (over limit)

        let actual = counter.maxed_out_tools();
        let expected = vec![read];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_zero_limit_maxes_out_immediately() {
        let read = &ToolName::new("READ");
        let mut counter = ToolErrorTracker::new(0);
        counter.adjust(&[read], &[]);

        let actual = counter.maxed_out_tools();
        let expected = vec![read];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_maxed_tool_cannot_recover_after_success() {
        let read = &ToolName::new("READ");
        let mut counter = ToolErrorTracker::new(2);
        counter
            .adjust(&[read], &[])
            .adjust(&[read], &[]) // Tool is now maxed out
            .adjust(&[], &[read]); // Success should remove from counts

        let actual = counter.maxed_out_tools();
        let expected: Vec<&ToolName> = vec![];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_single_tool_call() {
        let input = [ToolCallPart {
            call_id: Some(ToolCallId("call_1".to_string())),
            name: Some(ToolName::new("read")),
            arguments_part: "{\"path\": \"docs/onboarding.md\"}".to_string(),
            thought_signature: None,
            namespace: None,
        }];

        let actual = ToolCallFull::try_from_parts(&input).unwrap();
        let expected = vec![ToolCallFull {
            call_id: Some(ToolCallId("call_1".to_string())),
            name: ToolName::new("read"),
            arguments: ToolCallArguments::from_json(r#"{"path": "docs/onboarding.md"}"#),
            thought_signature: None,
            namespace: None,
        }];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_empty_call_parts() {
        let actual = ToolCallFull::try_from_parts(&[]).unwrap();
        let expected = vec![];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_empty_arguments() {
        let input = [ToolCallPart {
            call_id: Some(ToolCallId("call_1".to_string())),
            name: Some(ToolName::new("screenshot")),
            arguments_part: "".to_string(),
            thought_signature: None,
            namespace: None,
        }];

        let actual = ToolCallFull::try_from_parts(&input).unwrap();
        let expected = vec![ToolCallFull {
            call_id: Some(ToolCallId("call_1".to_string())),
            name: ToolName::new("screenshot"),
            arguments: ToolCallArguments::default(),
            thought_signature: None,
            namespace: None,
        }];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_real_example() {
        let message = forge_test_kit::fixture!("/src/fixtures/tool_call_01.md").await;

        let tool_call = ToolCallFull::try_from_xml(&message).unwrap();
        let actual = tool_call.first().unwrap().name.to_string();
        let expected = "attempt_completion";
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_try_from_xml_call_id() {
        let message = forge_test_kit::fixture!("/src/fixtures/tool_call_01.md").await;

        let tool_call = ToolCallFull::try_from_xml(&message).unwrap();
        let actual = tool_call.first().unwrap().call_id.as_ref().unwrap();
        assert!(actual.as_str().starts_with("forge_call_id_"));
    }
    #[test]
    fn test_try_from_parts_handles_empty_tool_names() {
        // Fixture: Tool call parts where empty names in subsequent parts should not
        // override valid names
        let input = [
            ToolCallPart {
                call_id: Some(ToolCallId("0".to_string())),
                name: Some(ToolName::new("read")),
                arguments_part: "".to_string(),
                thought_signature: None,
                namespace: None,
            },
            ToolCallPart {
                call_id: Some(ToolCallId("0".to_string())),
                name: Some(ToolName::new("")), // Empty name should not override valid name
                arguments_part: "{\"path\"".to_string(),
                thought_signature: None,
                namespace: None,
            },
            ToolCallPart {
                call_id: Some(ToolCallId("0".to_string())),
                name: Some(ToolName::new("")), // Empty name should not override valid name
                arguments_part: ": \"/test/file.md\"}".to_string(),
                thought_signature: None,
                namespace: None,
            },
        ];

        let actual = ToolCallFull::try_from_parts(&input).unwrap();
        let expected = vec![ToolCallFull {
            call_id: Some(ToolCallId("0".to_string())),
            name: ToolName::new("read"),
            arguments: ToolCallArguments::from_json(r#"{"path": "/test/file.md"}"#),
            thought_signature: None,
            namespace: None,
        }];

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_consecutive_failures_max_out_tool() {
        let read = &ToolName::new("READ");
        let mut counter = ToolErrorTracker::new(2);
        counter
            .adjust(&[read, read, read], &[])
            .adjust(&[read, read], &[])
            .adjust(&[read], &[]);

        let actual = counter.maxed_out_tools();
        let expected = vec![read];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_successful_tool_resets_then_other_tool_maxed_out() {
        let read = &ToolName::new("READ");
        let write = &ToolName::new("WRITE");
        let mut counter = ToolErrorTracker::new(2);
        counter
            .adjust(&[read, read, read], &[])
            .adjust(&[read, read], &[])
            .adjust(&[read], &[])
            .adjust(&[write], &[read])
            .adjust(&[write], &[])
            .adjust(&[write], &[]);

        let actual = counter.maxed_out_tools();
        let expected = vec![write];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_tool_maxed_out_despite_intermittent_successes() {
        let read = &ToolName::new("READ");
        let mut counter = ToolErrorTracker::new(2);
        counter
            .adjust(&[read, read, read], &[read])
            .adjust(&[read, read], &[read]) // Hitting limit
            .adjust(&[read], &[read]); // Still failing

        let actual = counter.maxed_out_tools();
        let expected = vec![read];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_tool_exactly_at_limit_is_maxed_out() {
        // Test that count == limit triggers maxed_out (testing >= not just >)
        let read = &ToolName::new("READ");
        let mut counter = ToolErrorTracker::new(3);
        counter
            .adjust(&[read], &[]) // count = 1
            .adjust(&[read], &[]) // count = 2
            .adjust(&[read], &[]); // count = 3 (exactly at limit)

        let actual = counter.maxed_out_tools();
        let expected = vec![read];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_tool_just_under_limit_not_maxed_out() {
        // Test that count < limit does NOT trigger maxed_out
        let read = &ToolName::new("READ");
        let mut counter = ToolErrorTracker::new(3);
        counter
            .adjust(&[read], &[]) // count = 1
            .adjust(&[read], &[]); // count = 2 (under limit of 3)

        let actual = counter.maxed_out_tools();
        let expected: Vec<&ToolName> = vec![];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_try_from_parts_preserves_thought_signature() {
        // Fixture: Tool call parts where first part has thought_signature
        let input = [
            ToolCallPart {
                call_id: Some(ToolCallId("call_1".to_string())),
                name: Some(ToolName::new("shell")),
                arguments_part: "{\"command\": \"date\"".to_string(),
                thought_signature: Some("signature_abc123".to_string()),
                namespace: None,
            },
            ToolCallPart {
                call_id: None,
                name: None,
                arguments_part: "}".to_string(),
                thought_signature: None, // Later parts typically don't have signature
                namespace: None,
            },
        ];

        let actual = ToolCallFull::try_from_parts(&input).unwrap();
        let expected = vec![ToolCallFull {
            call_id: Some(ToolCallId("call_1".to_string())),
            name: ToolName::new("shell"),
            arguments: ToolCallArguments::from_json(r#"{"command": "date"}"#),
            thought_signature: Some("signature_abc123".to_string()),
            namespace: None,
        }];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_try_from_parts_multiple_calls_with_thought_signatures() {
        // Fixture: Multiple tool calls where each has its own thought_signature
        let input = [
            ToolCallPart {
                call_id: Some(ToolCallId("call_1".to_string())),
                name: Some(ToolName::new("read")),
                arguments_part: "{\"path\": \"file1.txt\"}".to_string(),
                thought_signature: Some("sig_1".to_string()),
                namespace: None,
            },
            ToolCallPart {
                call_id: Some(ToolCallId("call_2".to_string())),
                name: Some(ToolName::new("read")),
                arguments_part: "{\"path\": \"file2.txt\"}".to_string(),
                thought_signature: Some("sig_2".to_string()),
                namespace: None,
            },
        ];

        let actual = ToolCallFull::try_from_parts(&input).unwrap();
        let expected = vec![
            ToolCallFull {
                call_id: Some(ToolCallId("call_1".to_string())),
                name: ToolName::new("read"),
                arguments: ToolCallArguments::from_json(r#"{"path": "file1.txt"}"#),
                thought_signature: Some("sig_1".to_string()),
                namespace: None,
            },
            ToolCallFull {
                call_id: Some(ToolCallId("call_2".to_string())),
                name: ToolName::new("read"),
                arguments: ToolCallArguments::from_json(r#"{"path": "file2.txt"}"#),
                thought_signature: Some("sig_2".to_string()),
                namespace: None,
            },
        ];

        assert_eq!(actual, expected);
    }
}
