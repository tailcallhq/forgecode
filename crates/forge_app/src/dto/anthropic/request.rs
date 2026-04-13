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

#[derive(Serialize, Default, Debug, PartialEq, Eq)]
pub struct Thinking {
    pub r#type: ThinkingType,
    pub budget_tokens: u64,
}

/// Effort level for Anthropic's `output_config` API.
///
/// Only the variants officially supported by Anthropic's `output_config.effort`
/// field. Mutually exclusive with the `thinking` object.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputEffort {
    Low,
    Medium,
    High,
    Max,
}

/// Output configuration for newer Anthropic models that support effort-based
/// reasoning (e.g. `claude-opus-4-6`).  Mutually exclusive with `thinking`.
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

#[derive(Serialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingType {
    #[default]
    Enabled,
    Disabled,
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

        // Route reasoning config to the correct Anthropic serialization.
        // All paths require enabled == Some(true); without it nothing is emitted.
        //
        // • enabled + max_tokens → thinking object (older models, e.g.
        // claude-3-7-sonnet).   An explicit reasoning budget unambiguously
        // selects the extended-thinking API.   effort (which may arrive from
        // embedded defaults) is ignored in this branch.
        //
        // • enabled + effort, no max_tokens → output_config.effort (newer models, e.g.
        //   claude-opus-4-6).  No token budget means the caller chose the effort-based
        // API.
        //
        // • enabled only (no effort, no max_tokens) → thinking with a default budget.
        let (thinking, output_config) = if let Some(reasoning) = request.reasoning {
            if reasoning.enabled == Some(true) {
                if let Some(budget) = reasoning.max_tokens {
                    // Explicit budget → thinking object regardless of effort.
                    (
                        Some(Thinking {
                            r#type: ThinkingType::Enabled,
                            budget_tokens: budget as u64,
                        }),
                        None,
                    )
                } else if let Some(effort) = reasoning.effort {
                    // Effort without budget → newer output_config API.
                    let output_effort = match effort {
                        forge_domain::Effort::Low => OutputEffort::Low,
                        forge_domain::Effort::High => OutputEffort::High,
                        forge_domain::Effort::Max => OutputEffort::Max,
                        // Map unsupported variants to the nearest Anthropic-valid effort.
                        forge_domain::Effort::None | forge_domain::Effort::Minimal => {
                            OutputEffort::Low
                        }
                        forge_domain::Effort::Medium => OutputEffort::Medium,
                        forge_domain::Effort::XHigh => OutputEffort::Max,
                    };
                    (None, Some(OutputConfig { effort: output_effort }))
                } else {
                    // Enabled-only → thinking with default budget.
                    (
                        Some(Thinking { r#type: ThinkingType::Enabled, budget_tokens: 10000 }),
                        None,
                    )
                }
            } else {
                // enabled=false or enabled=None → no reasoning emitted.
                (None, None)
            }
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
        {
            self.content[last_cacheable_idx] =
                std::mem::take(&mut self.content[last_cacheable_idx]).cached(true);
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
    fn test_thinking_type_serializes_to_enabled() {
        let thinking_type = ThinkingType::Enabled;
        let actual = serde_json::to_string(&thinking_type).unwrap();
        let expected = r#""enabled""#;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_thinking_type_serializes_to_disabled() {
        let thinking_type = ThinkingType::Disabled;
        let actual = serde_json::to_string(&thinking_type).unwrap();
        let expected = r#""disabled""#;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_thinking_struct_serializes_correctly() {
        let thinking = Thinking { r#type: ThinkingType::Enabled, budget_tokens: 5000 };
        let actual = serde_json::to_value(&thinking).unwrap();
        let expected = serde_json::json!({
            "type": "enabled",
            "budget_tokens": 5000
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_reasoning_enabled_with_max_tokens_creates_thinking() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(8000),
            effort: None,
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.thinking,
            Some(Thinking { r#type: ThinkingType::Enabled, budget_tokens: 8000 })
        );
        assert_eq!(actual.output_config, None);
    }

    #[test]
    fn test_reasoning_max_tokens_takes_priority_over_effort() {
        // When both max_tokens and effort are set, max_tokens triggers the thinking
        // path because an explicit budget means the caller wants the older API.
        let fixture = Context::default().reasoning(ReasoningConfig {
            effort: Some(forge_domain::Effort::Low),
            enabled: Some(true),
            max_tokens: Some(8000),
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.thinking,
            Some(Thinking { r#type: ThinkingType::Enabled, budget_tokens: 8000 })
        );
        assert_eq!(actual.output_config, None);
    }

    #[test]
    fn test_reasoning_effort_without_budget_creates_output_config() {
        // Effort with no max_tokens routes to output_config (newer model path).
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
        assert_eq!(actual.thinking, None);
    }

    #[test]
    fn test_reasoning_enabled_without_max_tokens_uses_default_budget() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: None,
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(
            actual.thinking,
            Some(Thinking { r#type: ThinkingType::Enabled, budget_tokens: 10000 })
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
    fn test_reasoning_enabled_none_does_not_create_thinking() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: None,
            max_tokens: Some(8000),
            effort: None,
            exclude: None,
        });

        let actual = Request::try_from(fixture).unwrap();

        assert_eq!(actual.thinking, None);
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
