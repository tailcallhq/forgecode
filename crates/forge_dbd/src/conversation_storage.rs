//! Write-side storage representation for conversation contexts.
//!
//! The DTOs below are the write closure of forge_repo's legacy
//! `ContextRecord` wire format. Keeping them here lets the write daemon
//! serialize the exact same bytes without depending on forge_repo.

use forge_domain::Context;
use serde::Serialize;

/// The context columns written by the conversation persistence path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedContext {
    pub context: Option<String>,
    pub message_count: Option<i32>,
    pub context_zstd: Option<Vec<u8>>,
    pub is_compressed: i32,
}

/// Converts a domain context into the legacy conversations-table columns.
pub fn persist_context(context: Option<&Context>) -> PersistedContext {
    let context =
        context.filter(|context| !context.messages.is_empty() || context.initiator.is_some());
    let message_count = context.map(|context| context.messages.len() as i32);
    let context_json = context
        .map(ContextRecord::from)
        .and_then(|context_record| serde_json::to_string(&context_record).ok());

    let (context, context_zstd, is_compressed) = if let Some(json) = context_json {
        match zstd::encode_all(json.as_bytes(), 3) {
            Ok(compressed) => (None, Some(compressed), 1),
            Err(_) => (Some(json), None, 0),
        }
    } else {
        (None, None, 0)
    };

    PersistedContext { context, message_count, context_zstd, is_compressed }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(transparent)]
struct ModelIdRecord(String);

impl From<&forge_domain::ModelId> for ModelIdRecord {
    fn from(id: &forge_domain::ModelId) -> Self {
        Self(id.to_string())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct ImageRecord {
    url: String,
    mime_type: String,
}

impl From<&forge_domain::Image> for ImageRecord {
    fn from(image: &forge_domain::Image) -> Self {
        Self {
            url: image.url().to_string(),
            mime_type: image.mime_type().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(transparent)]
struct ToolCallIdRecord(String);

impl From<&forge_domain::ToolCallId> for ToolCallIdRecord {
    fn from(id: &forge_domain::ToolCallId) -> Self {
        Self(id.as_str().to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
struct ToolCallArgumentsRecord(serde_json::Value);

impl From<&forge_domain::ToolCallArguments> for ToolCallArgumentsRecord {
    fn from(args: &forge_domain::ToolCallArguments) -> Self {
        Self(serde_json::to_value(args).unwrap_or_default())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(transparent)]
struct ToolNameRecord(String);

impl From<&forge_domain::ToolName> for ToolNameRecord {
    fn from(name: &forge_domain::ToolName) -> Self {
        Self(name.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
struct ToolCallFullRecord {
    name: ToolNameRecord,
    call_id: Option<ToolCallIdRecord>,
    arguments: ToolCallArgumentsRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    thought_signature: Option<String>,
}

impl From<&forge_domain::ToolCallFull> for ToolCallFullRecord {
    fn from(call: &forge_domain::ToolCallFull) -> Self {
        Self {
            name: ToolNameRecord::from(&call.name),
            call_id: call.call_id.as_ref().map(ToolCallIdRecord::from),
            arguments: ToolCallArgumentsRecord::from(&call.arguments),
            thought_signature: call.thought_signature.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct ReasoningFullRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    type_of: Option<String>,
}

impl From<&forge_domain::ReasoningFull> for ReasoningFullRecord {
    fn from(reasoning: &forge_domain::ReasoningFull) -> Self {
        Self {
            text: reasoning.text.clone(),
            signature: reasoning.signature.clone(),
            data: reasoning.data.clone(),
            id: reasoning.id.clone(),
            format: reasoning.format.clone(),
            index: reasoning.index,
            type_of: reasoning.type_of.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
enum TokenCountRecord {
    #[serde(alias = "Actual")]
    Actual(usize),
    #[serde(alias = "Approx")]
    Approx(usize),
}

impl From<&forge_domain::TokenCount> for TokenCountRecord {
    fn from(count: &forge_domain::TokenCount) -> Self {
        match count {
            forge_domain::TokenCount::Actual(n) => Self::Actual(*n),
            forge_domain::TokenCount::Approx(n) => Self::Approx(*n),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct UsageRecord {
    prompt_tokens: TokenCountRecord,
    completion_tokens: TokenCountRecord,
    total_tokens: TokenCountRecord,
    cached_tokens: TokenCountRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost: Option<f64>,
}

impl From<&forge_domain::Usage> for UsageRecord {
    fn from(usage: &forge_domain::Usage) -> Self {
        Self {
            prompt_tokens: TokenCountRecord::from(&usage.prompt_tokens),
            completion_tokens: TokenCountRecord::from(&usage.completion_tokens),
            total_tokens: TokenCountRecord::from(&usage.total_tokens),
            cached_tokens: TokenCountRecord::from(&usage.cached_tokens),
            cost: usage.cost,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
struct EventValueRecord(serde_json::Value);

impl From<&forge_domain::EventValue> for EventValueRecord {
    fn from(event: &forge_domain::EventValue) -> Self {
        Self(serde_json::to_value(event).unwrap_or_default())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
enum RoleRecord {
    System,
    User,
    Assistant,
    Tool,
}

impl From<&forge_domain::Role> for RoleRecord {
    fn from(role: &forge_domain::Role) -> Self {
        match role {
            forge_domain::Role::System => Self::System,
            forge_domain::Role::User => Self::User,
            forge_domain::Role::Assistant => Self::Assistant,
            forge_domain::Role::Tool => Self::Tool,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct TextMessageRecord {
    role: RoleRecord,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_content: Option<EventValueRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCallFullRecord>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thought_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<ModelIdRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_details: Option<Vec<ReasoningFullRecord>>,
    #[serde(default, skip_serializing_if = "is_false")]
    droppable: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl From<&forge_domain::TextMessage> for TextMessageRecord {
    fn from(message: &forge_domain::TextMessage) -> Self {
        Self {
            role: RoleRecord::from(&message.role),
            content: message.content.clone(),
            raw_content: message.raw_content.as_ref().map(EventValueRecord::from),
            tool_calls: message
                .tool_calls
                .as_ref()
                .map(|calls| calls.iter().map(ToolCallFullRecord::from).collect()),
            thought_signature: message.thought_signature.clone(),
            model: message.model.as_ref().map(ModelIdRecord::from),
            reasoning_details: message
                .reasoning_details
                .as_ref()
                .map(|details| details.iter().map(ReasoningFullRecord::from).collect()),
            droppable: message.droppable,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
enum ToolValueRecord {
    Text(String),
    AI {
        value: String,
        conversation_id: String,
    },
    Image(ImageRecord),
    Empty,
}

impl From<&forge_domain::ToolValue> for ToolValueRecord {
    fn from(value: &forge_domain::ToolValue) -> Self {
        match value {
            forge_domain::ToolValue::Text(text) => Self::Text(text.clone()),
            forge_domain::ToolValue::AI { value, conversation_id } => Self::AI {
                value: value.clone(),
                conversation_id: conversation_id.into_string(),
            },
            forge_domain::ToolValue::Image(image) => Self::Image(ImageRecord::from(image)),
            forge_domain::ToolValue::Empty => Self::Empty,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ToolOutputRecord {
    is_error: bool,
    values: Vec<ToolValueRecord>,
}

impl From<&forge_domain::ToolOutput> for ToolOutputRecord {
    fn from(output: &forge_domain::ToolOutput) -> Self {
        Self {
            is_error: output.is_error,
            values: output.values.iter().map(ToolValueRecord::from).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ToolResultRecord {
    name: ToolNameRecord,
    call_id: Option<ToolCallIdRecord>,
    output: ToolOutputRecord,
}

impl From<&forge_domain::ToolResult> for ToolResultRecord {
    fn from(result: &forge_domain::ToolResult) -> Self {
        Self {
            name: ToolNameRecord::from(&result.name),
            call_id: result.call_id.as_ref().map(ToolCallIdRecord::from),
            output: ToolOutputRecord::from(&result.output),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum ContextMessageValueRecord {
    Text(TextMessageRecord),
    Tool(ToolResultRecord),
    Image(ImageRecord),
}

impl From<&forge_domain::ContextMessage> for ContextMessageValueRecord {
    fn from(value: &forge_domain::ContextMessage) -> Self {
        match value {
            forge_domain::ContextMessage::Text(message) => {
                Self::Text(TextMessageRecord::from(message))
            }
            forge_domain::ContextMessage::Tool(result) => {
                Self::Tool(ToolResultRecord::from(result))
            }
            forge_domain::ContextMessage::Image(image) => Self::Image(ImageRecord::from(image)),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ContextMessageRecord {
    message: ContextMessageValueRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<UsageRecord>,
}

impl From<&forge_domain::MessageEntry> for ContextMessageRecord {
    fn from(message: &forge_domain::MessageEntry) -> Self {
        Self {
            message: ContextMessageValueRecord::from(&message.message),
            usage: message.usage.as_ref().map(UsageRecord::from),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ToolDefinitionRecord {
    name: ToolNameRecord,
    description: String,
    input_schema: serde_json::Value,
}

impl From<&forge_domain::ToolDefinition> for ToolDefinitionRecord {
    fn from(definition: &forge_domain::ToolDefinition) -> Self {
        Self {
            name: ToolNameRecord::from(&definition.name),
            description: definition.description.clone(),
            input_schema: serde_json::to_value(&definition.input_schema).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
enum ToolChoiceRecord {
    None,
    Auto,
    Required,
    Call(ToolNameRecord),
}

impl From<&forge_domain::ToolChoice> for ToolChoiceRecord {
    fn from(choice: &forge_domain::ToolChoice) -> Self {
        match choice {
            forge_domain::ToolChoice::None => Self::None,
            forge_domain::ToolChoice::Auto => Self::Auto,
            forge_domain::ToolChoice::Required => Self::Required,
            forge_domain::ToolChoice::Call(name) => Self::Call(ToolNameRecord::from(name)),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
enum EffortRecord {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl From<&forge_domain::Effort> for EffortRecord {
    fn from(effort: &forge_domain::Effort) -> Self {
        match effort {
            forge_domain::Effort::None => Self::None,
            forge_domain::Effort::Minimal => Self::Minimal,
            forge_domain::Effort::Low => Self::Low,
            forge_domain::Effort::Medium => Self::Medium,
            forge_domain::Effort::High => Self::High,
            forge_domain::Effort::XHigh => Self::XHigh,
            forge_domain::Effort::Max => Self::Max,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ReasoningConfigRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<EffortRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exclude: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
}

impl From<&forge_domain::ReasoningConfig> for ReasoningConfigRecord {
    fn from(config: &forge_domain::ReasoningConfig) -> Self {
        Self {
            effort: config.effort.as_ref().map(EffortRecord::from),
            max_tokens: config.max_tokens,
            exclude: config.exclude,
            enabled: config.enabled,
        }
    }
}

/// Repository-compatible representation of `Context` for writes only.
#[derive(Debug, Clone, Serialize)]
struct ContextRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    initiator: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    messages: Vec<ContextMessageRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolDefinitionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_choice: Option<ToolChoiceRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfigRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

impl From<&Context> for ContextRecord {
    fn from(context: &Context) -> Self {
        Self {
            conversation_id: context.conversation_id.as_ref().map(|id| id.into_string()),
            initiator: context.initiator.clone(),
            messages: context
                .messages
                .iter()
                .map(ContextMessageRecord::from)
                .collect(),
            tools: context
                .tools
                .iter()
                .map(ToolDefinitionRecord::from)
                .collect(),
            tool_choice: context.tool_choice.as_ref().map(ToolChoiceRecord::from),
            max_tokens: context.max_tokens,
            temperature: context.temperature.map(|temperature| temperature.value()),
            top_p: context.top_p.map(|top_p| top_p.value()),
            top_k: context.top_k.map(|top_k| top_k.value()),
            reasoning: context.reasoning.as_ref().map(ReasoningConfigRecord::from),
            stream: context.stream,
        }
    }
}
