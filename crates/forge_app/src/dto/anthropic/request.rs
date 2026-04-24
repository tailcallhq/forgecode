use derive_setters::Setters;
use forge_domain::{ContextMessage, Image};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Default, Setters)]
#[setters(into, strip_option)]
pub struct Request {
    pub max_tokens: u64,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<SystemMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Thinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<OutputConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_format: Option<OutputFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_version: Option<String>,
}

#[derive(Serialize, Default)]
pub struct SystemMessage {
    pub r#type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl SystemMessage {
    pub fn cached(mut self, cached: bool) -> Self {
        self.cache_control = if cached {
            Some(CacheControl::Ephemeral)
        } else {
            None
        };
        self
    }

    pub fn is_cached(&self) -> bool {
        self.cache_control.is_some()
    }
}

/// Anthropic's `thinking` request field. Opus 4.7 rejects the `Enabled` shape
/// and the orchestrator applies model-specific reasoning normalization before
/// request conversion.
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Thinking {
    Enabled {
        budget_tokens: u64,
    },
    Adaptive {
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<ThinkingDisplay>,
    },
    Disabled,
}

/// On Opus 4.7 adaptive thinking content is omitted from responses unless
/// `Summarized` is requested explicitly.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingDisplay {
    Summarized,
    Omitted,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputEffort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct OutputConfig {
    pub effort: OutputEffort,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputFormat {
    #[serde(rename = "json_schema")]
    JsonSchema { schema: schemars::Schema },
}

impl TryFrom<forge_domain::Context> for Request {
    type Error = anyhow::Error;
    fn try_from(request: forge_domain::Context) -> std::result::Result<Self, Self::Error> {
        let system_messages = request
            .messages
            .iter()
            .filter_map(|msg| match &**msg {
                ContextMessage::Text(msg) if msg.has_role(forge_domain::Role::System) => {
                    Some(SystemMessage {
                        r#type: "text".to_string(),
                        text: msg.content.clone(),
                        cache_control: None,
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        // Gate on the domain rule so inherited configs with `enabled: None` but
        // a positive effort / `max_tokens` still emit reasoning on the wire.
        let reasoning_on = request.is_reasoning_supported();
        let (thinking, output_config) = if reasoning_on && let Some(reasoning) = request.reasoning {
            // Adaptive thinking on 4.7 hides reasoning content by default; opting
            // into reasoning should surface it unless the caller set `exclude`.
            let adaptive_display = if reasoning.exclude == Some(true) {
                Some(ThinkingDisplay::Omitted)
            } else {
                Some(ThinkingDisplay::Summarized)
            };

            let thinking = if let Some(budget) = reasoning.max_tokens {
                Thinking::Enabled { budget_tokens: budget as u64 }
            } else {
                Thinking::Adaptive { display: adaptive_display }
            };

            // `Effort::None` is an explicit opt-out; `is_reasoning_supported`
            // already filters it, but guard here so it can never become a stray
            // `output_config.effort`.
            let output_config = reasoning.effort.and_then(|effort| {
                let output_effort = match effort {
                    forge_domain::Effort::None => return None,
                    forge_domain::Effort::Minimal | forge_domain::Effort::Low => OutputEffort::Low,
                    forge_domain::Effort::Medium => OutputEffort::Medium,
                    forge_domain::Effort::High => OutputEffort::High,
                    forge_domain::Effort::XHigh => OutputEffort::XHigh,
                    forge_domain::Effort::Max => OutputEffort::Max,
                };
                Some(OutputConfig { effort: output_effort })
            });

            (Some(thinking), output_config)
        } else {
            (None, None)
        };

        Ok(Self {
            messages: request
                .messages
                .into_iter()
                .filter(|message| !message.has_role(forge_domain::Role::System))
                .map(|msg| Message::try_from(msg.message))
                .collect::<std::result::Result<Vec<_>, _>>()?,
            tools: request
                .tools
                .into_iter()
                .map(ToolDefinition::try_from)
                .collect::<std::result::Result<Vec<_>, _>>()?,
            system: Some(system_messages),
            temperature: request.temperature.map(|t| t.value()),
            top_p: request.top_p.map(|t| t.value()),
            top_k: request.top_k.map(|t| t.value() as u64),
            tool_choice: request.tool_choice.map(ToolChoice::from),
            stream: Some(request.stream.unwrap_or(true)),
            thinking,
            output_config,
            output_format: request.response_format.and_then(|rf| match rf {
                forge_domain::ResponseFormat::Text => {
                    // Anthropic doesn't have a "text" output format, so we skip it
                    None
                }
                forge_domain::ResponseFormat::JsonSchema(schema) => {
                    Some(OutputFormat::JsonSchema { schema: *schema })
                }
            }),
            ..Default::default()
        })
    }
}

impl Request {
    /// Get a reference to the messages
    pub fn get_messages(&self) -> &[Message] {
        &self.messages
    }

    /// Get a mutable reference to the messages
    pub fn get_messages_mut(&mut self) -> &mut Vec<Message> {
        &mut self.messages
    }
}

#[derive(Serialize)]
pub struct Metadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

#[derive(Serialize)]
pub struct Message {
    pub content: Vec<Content>,
    pub role: Role,
}

impl TryFrom<ContextMessage> for Message {
    type Error = anyhow::Error;
    fn try_from(value: ContextMessage) -> std::result::Result<Self, Self::Error> {
        Ok(match value {
            ContextMessage::Text(chat_message) => {
                let mut content = Vec::with_capacity(
                    chat_message
                        .tool_calls
                        .as_ref()
                        .map(|tc| tc.len())
                        .unwrap_or_default()
                        + 1,
                );

                if let Some(reasoning) = chat_message.reasoning_details
                    && let Some((sig, text)) = reasoning.into_iter().find_map(|reasoning| {
                        match (reasoning.signature, reasoning.text) {
                            (Some(sig), Some(text)) => Some((sig, text)),
                            _ => None,
                        }
                    })
                {
                    content.push(Content::Thinking { signature: Some(sig), thinking: Some(text) });
                }

                if !chat_message.content.is_empty() {
                    // NOTE: Anthropic does not allow empty text content.
                    content.push(Content::Text { text: chat_message.content, cache_control: None });
                }
                if let Some(tool_calls) = chat_message.tool_calls {
                    for tool_call in tool_calls {
                        content.push(tool_call.try_into()?);
                    }
                }

                match chat_message.role {
                    forge_domain::Role::User => Message { role: Role::User, content },
                    forge_domain::Role::Assistant => Message { role: Role::Assistant, content },
                    forge_domain::Role::System => {
                        // note: Anthropic doesn't support system role messages and they're already
                        // filtered out. so this state is unreachable.
                        return Err(
                            forge_domain::Error::UnsupportedRole("System".to_string()).into()
                        );
                    }
                }
            }
            ContextMessage::Tool(tool_result) => {
                Message { role: Role::User, content: vec![tool_result.try_into()?] }
            }
            ContextMessage::Image(img) => {
                Message { content: vec![Content::from(img)], role: Role::User }
            }
        })
    }
}

impl Message {
    pub fn cached(mut self, enable_cache: bool) -> Self {
        // Reset cache control on all content items first
        for content in &mut self.content {
            *content = std::mem::take(content).cached(false);
        }

        // If enabling cache, set cache control on the last cacheable content item
        if enable_cache
            && let Some(last_cacheable_idx) =
                self.content
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(idx, content)| match content {
                        Content::Text { .. }
                        | Content::Image { .. }
                        | Content::ToolUse { .. }
                        | Content::ToolResult { .. } => Some(idx),
                        _ => None,
                    })
            && let Some(content) = self.content.get_mut(last_cacheable_idx)
        {
            *content = std::mem::take(content).cached(true);
        }

        self
    }

    pub fn is_cached(&self) -> bool {
        self.content.iter().any(|content| content.is_cached())
    }
}

impl Default for Message {
    fn default() -> Self {
        Message { content: vec![], role: Role::User }
    }
}

impl From<Image> for Content {
    fn from(value: Image) -> Self {
        Content::Image {
            source: ImageSource {
                type_: "base64".to_string(),
                media_type: Some(value.mime_type().to_string()),
                data: Some(value.data().into()),
                url: None,
            },
            cache_control: None,
        }
    }
}

#[derive(Serialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Content {
    Image {
        source: ImageSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    ToolUse {
        id: String,
        input: Option<serde_json::Value>,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    Thinking {
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking: Option<String>,
    },
}

impl Default for Content {
    fn default() -> Self {
        Content::Thinking { signature: None, thinking: None }
    }
}

impl Content {
    pub fn cached(self, enable_cache: bool) -> Self {
        let cache_control = enable_cache.then_some(CacheControl::Ephemeral);

        match self {
            Content::Text { text, .. } => Content::Text { text, cache_control },
            Content::ToolUse { id, input, name, .. } => {
                Content::ToolUse { id, input, name, cache_control }
            }
            Content::ToolResult { tool_use_id, content, is_error, .. } => {
                Content::ToolResult { tool_use_id, content, is_error, cache_control }
            }
            Content::Image { source, .. } => Content::Image { source, cache_control },
            // TODO: verify this Thinking variants don't support cache control
            Content::Thinking { signature, thinking } => Content::Thinking { signature, thinking },
        }
    }

    pub fn is_cached(&self) -> bool {
        match self {
            Content::Text { cache_control, .. } => cache_control.is_some(),
            Content::ToolUse { cache_control, .. } => cache_control.is_some(),
            Content::ToolResult { cache_control, .. } => cache_control.is_some(),
            Content::Image { cache_control, .. } => cache_control.is_some(),
            Content::Thinking { .. } => false,
        }
    }
}

impl TryFrom<forge_domain::ToolCallFull> for Content {
    type Error = anyhow::Error;
    fn try_from(value: forge_domain::ToolCallFull) -> std::result::Result<Self, Self::Error> {
        let call_id = value
            .call_id
            .as_ref()
            .ok_or(forge_domain::Error::ToolCallMissingId)?;

        Ok(Content::ToolUse {
            id: call_id.as_str().to_string(),
            input: serde_json::to_value(value.arguments).ok(),
            name: value.name.to_string(),
            cache_control: None,
        })
    }
}

impl TryFrom<forge_domain::ToolResult> for Content {
    type Error = anyhow::Error;
    fn try_from(value: forge_domain::ToolResult) -> std::result::Result<Self, Self::Error> {
        let call_id = value
            .call_id
            .as_ref()
            .ok_or(forge_domain::Error::ToolCallMissingId)?;
        Ok(Content::ToolResult {
            tool_use_id: call_id.as_str().to_string(),
            cache_control: None,
            content: value
                .output
                .values
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .next(),
            is_error: Some(value.is_error()),
        })
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CacheControl {
    Ephemeral,
}

#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ToolChoice {
    Auto {
        #[serde(skip_serializing_if = "Option::is_none")]
        disable_parallel_tool_use: Option<bool>,
    },
    Any {
        #[serde(skip_serializing_if = "Option::is_none")]
        disable_parallel_tool_use: Option<bool>,
    },
    Tool {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        disable_parallel_tool_use: Option<bool>,
    },
}

// To understand the mappings refer: https://docs.anthropic.com/en/docs/build-with-claude/tool-use#controlling-claudes-output
impl From<forge_domain::ToolChoice> for ToolChoice {
    fn from(value: forge_domain::ToolChoice) -> Self {
        match value {
            forge_domain::ToolChoice::Auto => ToolChoice::Auto { disable_parallel_tool_use: None },
            forge_domain::ToolChoice::Call(tool_name) => {
                ToolChoice::Tool { name: tool_name.to_string(), disable_parallel_tool_use: None }
            }
            forge_domain::ToolChoice::Required => {
                ToolChoice::Any { disable_parallel_tool_use: None }
            }
            forge_domain::ToolChoice::None => ToolChoice::Auto { disable_parallel_tool_use: None },
        }
    }
}

#[derive(Serialize)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
    pub input_schema: serde_json::Value,
}

impl TryFrom<forge_domain::ToolDefinition> for ToolDefinition {
    type Error = anyhow::Error;
    fn try_from(value: forge_domain::ToolDefinition) -> std::result::Result<Self, Self::Error> {
        Ok(ToolDefinition {
            name: value.name.to_string(),
            description: Some(value.description),
            cache_control: None,
            input_schema: serde_json::to_value(value.input_schema)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, ReasoningConfig};
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_thinking_enabled_serializes_with_budget() {
        let thinking = Thinking::Enabled { budget_tokens: 5000 };
        let actual = serde_json::to_value(&thinking).unwrap();
        let expected = serde_json::json!({
            "type": "enabled",
            "budget_tokens": 5000
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_thinking_adaptive_serializes_without_display_when_none() {
        let thinking = Thinking::Adaptive { display: None };
        let actual = serde_json::to_value(&thinking).unwrap();
        let expected = serde_json::json!({"type": "adaptive"});

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_thinking_adaptive_serializes_with_summarized_display() {
        let thinking = Thinking::Adaptive { display: Some(ThinkingDisplay::Summarized) };
        let actual = serde_json::to_value(&thinking).unwrap();
        let expected = serde_json::json!({
            "type": "adaptive",
            "display": "summarized"
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_thinking_disabled_serializes() {
        let thinking = Thinking::Disabled;
        let actual = serde_json::to_value(&thinking).unwrap();
        let expected = serde_json::json!({"type": "disabled"});

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_reasoning_enabled_with_max_tokens_creates_enabled_thinking() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(8000),
            effort: None,
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.thinking,
            Some(Thinking::Enabled { budget_tokens: 8000 })
        );
        assert_eq!(actual.output_config, None);
    }

    #[test]
    fn test_reasoning_max_tokens_and_effort_emit_both() {
        // Effort and budget are independent knobs — neither should hide the other.
        let fixture = Context::default().reasoning(ReasoningConfig {
            effort: Some(forge_domain::Effort::Low),
            enabled: Some(true),
            max_tokens: Some(8000),
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.thinking,
            Some(Thinking::Enabled { budget_tokens: 8000 })
        );
        assert_eq!(
            actual.output_config,
            Some(OutputConfig { effort: OutputEffort::Low })
        );
    }

    #[test]
    fn test_reasoning_max_tokens_alone_emits_enabled_only() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            effort: None,
            enabled: Some(true),
            max_tokens: Some(8000),
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.thinking,
            Some(Thinking::Enabled { budget_tokens: 8000 })
        );
        assert_eq!(actual.output_config, None);
    }

    #[test]
    fn test_reasoning_effort_without_budget_creates_output_config_and_adaptive() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            effort: Some(forge_domain::Effort::Low),
            enabled: Some(true),
            max_tokens: None,
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.output_config,
            Some(OutputConfig { effort: OutputEffort::Low })
        );
        assert_eq!(
            actual.thinking,
            Some(Thinking::Adaptive { display: Some(ThinkingDisplay::Summarized) })
        );
    }

    #[test]
    fn test_reasoning_effort_with_exclude_emits_adaptive_omitted() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            effort: Some(forge_domain::Effort::High),
            enabled: Some(true),
            max_tokens: None,
            exclude: Some(true),
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.thinking,
            Some(Thinking::Adaptive { display: Some(ThinkingDisplay::Omitted) })
        );
    }

    #[test]
    fn test_reasoning_xhigh_effort_maps_to_xhigh() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            effort: Some(forge_domain::Effort::XHigh),
            enabled: Some(true),
            max_tokens: None,
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.output_config,
            Some(OutputConfig { effort: OutputEffort::XHigh })
        );
    }

    #[test]
    fn test_reasoning_enabled_without_budget_or_effort_defaults_to_adaptive_summarized() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: None,
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.thinking,
            Some(Thinking::Adaptive { display: Some(ThinkingDisplay::Summarized) })
        );
    }

    #[test]
    fn test_reasoning_enabled_with_exclude_uses_omitted_display() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: None,
            exclude: Some(true),
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.thinking,
            Some(Thinking::Adaptive { display: Some(ThinkingDisplay::Omitted) })
        );
    }

    #[test]
    fn test_reasoning_disabled_does_not_create_thinking() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(false),
            max_tokens: Some(8000),
            effort: None,
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(actual.thinking, None);
    }

    #[test]
    fn test_reasoning_enabled_none_with_max_tokens_still_emits_thinking() {
        // Matches the domain's `is_reasoning_supported` rule: enabled: None with a
        // positive budget counts as on, so inherited/merged configs don't silently
        // disable reasoning on the wire.
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: None,
            max_tokens: Some(8000),
            effort: None,
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.thinking,
            Some(Thinking::Enabled { budget_tokens: 8000 })
        );
    }

    #[test]
    fn test_reasoning_enabled_none_with_effort_still_emits_output_config() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: None,
            max_tokens: None,
            effort: Some(forge_domain::Effort::High),
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.output_config,
            Some(OutputConfig { effort: OutputEffort::High })
        );
        assert_eq!(
            actual.thinking,
            Some(Thinking::Adaptive { display: Some(ThinkingDisplay::Summarized) })
        );
    }

    #[test]
    fn test_reasoning_enabled_none_with_zero_max_tokens_does_not_emit() {
        // Matches `is_reasoning_supported`: max_tokens > 0 is required.
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: None,
            max_tokens: Some(0),
            effort: None,
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(actual.thinking, None);
        assert_eq!(actual.output_config, None);
    }

    #[test]
    fn test_reasoning_effort_none_does_not_emit_anything() {
        // Effort::None is an explicit opt-out — no thinking, no output_config.
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: None,
            max_tokens: None,
            effort: Some(forge_domain::Effort::None),
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(actual.thinking, None);
        assert_eq!(actual.output_config, None);
    }

    #[test]
    fn test_reasoning_effort_none_overrides_enabled_and_max_tokens() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(8000),
            effort: Some(forge_domain::Effort::None),
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(actual.thinking, None);
        assert_eq!(actual.output_config, None);
    }

    #[test]
    fn test_reasoning_enabled_false_overrides_effort() {
        // Explicit opt-out beats inferred enablement.
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(false),
            max_tokens: None,
            effort: Some(forge_domain::Effort::High),
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(actual.thinking, None);
        assert_eq!(actual.output_config, None);
    }

    #[test]
    fn test_no_reasoning_config_does_not_create_thinking() {
        let fixture = Context::default();

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(actual.thinking, None);
    }

    #[test]
    fn test_context_conversion_stream_defaults_to_true() {
        let fixture = Context::default();
        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(actual.stream, Some(true));
    }

    #[test]
    fn test_context_conversion_stream_explicit_true() {
        let fixture = Context::default().stream(true);
        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(actual.stream, Some(true));
    }

    #[test]
    fn test_context_conversion_stream_explicit_false() {
        let fixture = Context::default().stream(false);
        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(actual.stream, Some(false));
    }
}
