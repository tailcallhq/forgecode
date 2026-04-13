use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamInput;
use aws_sdk_bedrockruntime::types::ContentBlock;
use forge_domain::Transformer;
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    /// Regex for identifying characters that are invalid in Bedrock tool call IDs.
    /// Bedrock (especially with Anthropic models) requires tool call IDs to match
    /// the pattern `^[a-zA-Z0-9_-]+$`. This regex matches any character that is
    /// NOT alphanumeric, underscore, or hyphen.
    static ref INVALID_CHARS: Regex = Regex::new(r"[^a-zA-Z0-9_-]").unwrap();
}

/// Transformer that sanitizes tool call IDs for Bedrock compatibility.
///
/// Bedrock (especially with Anthropic models) requires tool call IDs to match
/// the pattern `^[a-zA-Z0-9_-]+$`. This transformer replaces any invalid
/// characters (non-alphanumeric, non-underscore, non-hyphen) with underscores.
///
/// This addresses the ValidationException error:
/// "messages.1.content.0.tool_use.id: String should match pattern
/// '^[a-zA-Z0-9_-]+$'"
pub struct SanitizeToolIds;

impl Transformer for SanitizeToolIds {
    type Value = ConverseStreamInput;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        // Use the pre-compiled regex for efficient sanitization

        // Sanitize tool_use_id in messages
        if let Some(messages) = request.messages.as_mut() {
            for message in messages.iter_mut() {
                let mut new_content = Vec::with_capacity(message.content.len());
                for content_block in message.content.drain(..) {
                    let new_block = match content_block {
                        ContentBlock::ToolUse(tool_use) => {
                            let sanitized_id = INVALID_CHARS
                                .replace_all(tool_use.tool_use_id(), "_")
                                .to_string();
                            // Rebuild ToolUseBlock with sanitized ID
                            let rebuilt = aws_sdk_bedrockruntime::types::ToolUseBlock::builder()
                                .tool_use_id(sanitized_id)
                                .name(tool_use.name().to_string())
                                .input(tool_use.input().clone())
                                .build()
                                .expect("Failed to rebuild ToolUseBlock");
                            ContentBlock::ToolUse(rebuilt)
                        }
                        ContentBlock::ToolResult(tool_result) => {
                            let sanitized_id = INVALID_CHARS
                                .replace_all(tool_result.tool_use_id(), "_")
                                .to_string();
                            // Rebuild ToolResultBlock with sanitized ID
                            let mut builder =
                                aws_sdk_bedrockruntime::types::ToolResultBlock::builder()
                                    .tool_use_id(sanitized_id);
                            // Copy content (returns a slice, not Option)
                            let content = tool_result.content();
                            if !content.is_empty() {
                                builder = builder.set_content(Some(content.to_vec()));
                            }
                            // Copy status if present
                            if let Some(status) = tool_result.status() {
                                builder = builder.status(status.clone());
                            }
                            let rebuilt =
                                builder.build().expect("Failed to rebuild ToolResultBlock");
                            ContentBlock::ToolResult(rebuilt)
                        }
                        other => other,
                    };
                    new_content.push(new_block);
                }
                message.content = new_content;
            }
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use aws_sdk_bedrockruntime::types::ContentBlock;
    use forge_domain::{
        Context, ContextMessage, Role, TextMessage, ToolCallArguments, ToolCallFull, ToolCallId,
        ToolName, ToolResult,
    };

    use super::*;
    use crate::provider::FromDomain;

    #[test]
    fn test_sanitizes_tool_use_id_with_colon_and_dot() {
        // This is the exact error case from the issue: "functions.shell:0"
        let context = Context {
            conversation_id: None,
            messages: vec![
                ContextMessage::Text(
                    TextMessage::new(Role::Assistant, "test")
                        .tool_calls(vec![
                            ToolCallFull::new(ToolName::new("shell"))
                                .call_id("functions.shell:0")
                                .arguments(ToolCallArguments::from_json("{}")),
                        ])
                        .model(forge_domain::ModelId::new("test")),
                )
                .into(),
            ],
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            reasoning: None,
            stream: None,
            response_format: None,
            initiator: None,
        };

        let request = ConverseStreamInput::from_domain(context).expect("Failed to convert context");
        let mut transformer = SanitizeToolIds;
        let actual = transformer.transform(request);

        // Find the ToolUse content block by searching all messages and content
        let tool_use_id = actual.messages.as_ref().and_then(|msgs| {
            msgs.iter().find_map(|msg| {
                msg.content.iter().find_map(|block| {
                    if let ContentBlock::ToolUse(tool_use) = block {
                        Some(tool_use.tool_use_id().to_string())
                    } else {
                        None
                    }
                })
            })
        });

        assert_eq!(tool_use_id, Some("functions_shell_0".to_string()));
    }

    #[test]
    fn test_sanitizes_tool_result_id_with_invalid_chars() {
        let context = Context {
            conversation_id: None,
            messages: vec![
                ContextMessage::tool_result(
                    ToolResult::new(ToolName::new("test_tool"))
                        .call_id(ToolCallId::new("toolu_01!@#$ABC123"))
                        .success("result"),
                )
                .into(),
            ],
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            reasoning: None,
            stream: None,
            response_format: None,
            initiator: None,
        };

        let request = ConverseStreamInput::from_domain(context).expect("Failed to convert context");
        let mut transformer = SanitizeToolIds;
        let actual = transformer.transform(request);

        // Find the ToolResult content block
        let tool_result_id = actual
            .messages
            .as_ref()
            .and_then(|msgs| msgs.first())
            .and_then(|msg| msg.content.first())
            .and_then(|block| {
                if let ContentBlock::ToolResult(tool_result) = block {
                    Some(tool_result.tool_use_id().to_string())
                } else {
                    None
                }
            });

        assert_eq!(tool_result_id, Some("toolu_01____ABC123".to_string()));
    }

    #[test]
    fn test_leaves_valid_tool_ids_unchanged() {
        let valid_id = "call_abc-123_XYZ";
        let context = Context {
            conversation_id: None,
            messages: vec![
                ContextMessage::Text(
                    TextMessage::new(Role::Assistant, "test")
                        .tool_calls(vec![
                            ToolCallFull::new(ToolName::new("test_tool"))
                                .call_id(valid_id)
                                .arguments(ToolCallArguments::from_json("{}")),
                        ])
                        .model(forge_domain::ModelId::new("test")),
                )
                .into(),
            ],
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            reasoning: None,
            stream: None,
            response_format: None,
            initiator: None,
        };

        let request = ConverseStreamInput::from_domain(context).expect("Failed to convert context");
        let mut transformer = SanitizeToolIds;
        let actual = transformer.transform(request);

        // Find the ToolUse content block by searching all messages and content
        let tool_use_id = actual.messages.as_ref().and_then(|msgs| {
            msgs.iter().find_map(|msg| {
                msg.content.iter().find_map(|block| {
                    if let ContentBlock::ToolUse(tool_use) = block {
                        Some(tool_use.tool_use_id().to_string())
                    } else {
                        None
                    }
                })
            })
        });

        assert_eq!(tool_use_id, Some(valid_id.to_string()));
    }

    #[test]
    fn test_no_panic_on_empty_messages() {
        let context = Context {
            conversation_id: None,
            messages: vec![],
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            reasoning: None,
            stream: None,
            response_format: None,
            initiator: None,
        };

        let request = ConverseStreamInput::from_domain(context).expect("Failed to convert context");
        let mut transformer = SanitizeToolIds;
        let _ = transformer.transform(request); // Should not panic
    }
}
