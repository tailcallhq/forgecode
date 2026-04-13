//! Data Transfer Objects for Conversation Repository
//!
//! This module contains repository-specific record types that mirror their
//! `forge_domain` counterparts for compile-time safety while keeping the
//! storage layer independent from domain model changes.

use anyhow::Context as _;
use forge_domain::{Context, ConversationId};
use serde::{Deserialize, Serialize};

/// Repository-specific representation of ModelId
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub(super) struct ModelIdRecord(String);

impl From<&forge_domain::ModelId> for ModelIdRecord {
    fn from(id: &forge_domain::ModelId) -> Self {
        Self(id.to_string())
    }
}

impl From<ModelIdRecord> for forge_domain::ModelId {
    fn from(record: ModelIdRecord) -> Self {
        forge_domain::ModelId::from(record.0)
    }
}

/// Repository-specific representation of Image
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct ImageRecord {
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

impl From<ImageRecord> for forge_domain::Image {
    fn from(record: ImageRecord) -> Self {
        forge_domain::Image::new_base64(
            record
                .url
                .strip_prefix(&format!("data:{};base64,", record.mime_type))
                .unwrap_or(&record.url)
                .to_string(),
            record.mime_type,
        )
    }
}

/// Repository-specific representation of ToolCallId
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub(super) struct ToolCallIdRecord(String);

impl From<&forge_domain::ToolCallId> for ToolCallIdRecord {
    fn from(id: &forge_domain::ToolCallId) -> Self {
        Self(id.as_str().to_string())
    }
}

impl From<ToolCallIdRecord> for forge_domain::ToolCallId {
    fn from(record: ToolCallIdRecord) -> Self {
        forge_domain::ToolCallId::new(record.0)
    }
}

/// Repository-specific representation of ToolCallArguments
/// Stored as raw JSON Value to handle both parsed and unparsed variants
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct ToolCallArgumentsRecord(serde_json::Value);

impl From<&forge_domain::ToolCallArguments> for ToolCallArgumentsRecord {
    fn from(args: &forge_domain::ToolCallArguments) -> Self {
        // Serialize to JSON to capture both Parsed and Unparsed variants
        Self(serde_json::to_value(args).unwrap_or_default())
    }
}

impl From<ToolCallArgumentsRecord> for forge_domain::ToolCallArguments {
    fn from(record: ToolCallArgumentsRecord) -> Self {
        // Deserialize back to ToolCallArguments (always becomes Parsed variant)
        serde_json::from_value(record.0).unwrap_or_default()
    }
}

/// Repository-specific representation of ToolName
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub(super) struct ToolNameRecord(String);

impl From<&forge_domain::ToolName> for ToolNameRecord {
    fn from(name: &forge_domain::ToolName) -> Self {
        Self(name.to_string())
    }
}

impl From<ToolNameRecord> for forge_domain::ToolName {
    fn from(record: ToolNameRecord) -> Self {
        forge_domain::ToolName::new(record.0)
    }
}

/// Repository-specific representation of ToolCallFull
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ToolCallFullRecord {
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

impl From<ToolCallFullRecord> for forge_domain::ToolCallFull {
    fn from(record: ToolCallFullRecord) -> Self {
        forge_domain::ToolCallFull {
            name: record.name.into(),
            call_id: record.call_id.map(Into::into),
            arguments: record.arguments.into(),
            thought_signature: record.thought_signature,
        }
    }
}

/// Repository-specific representation of ReasoningFull (alias for
/// ReasoningDetail)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct ReasoningFullRecord {
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

impl From<ReasoningFullRecord> for forge_domain::ReasoningFull {
    fn from(record: ReasoningFullRecord) -> Self {
        forge_domain::ReasoningFull {
            text: record.text,
            signature: record.signature,
            data: record.data,
            id: record.id,
            format: record.format,
            index: record.index,
            type_of: record.type_of,
        }
    }
}

/// Repository-specific representation of TokenCount
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum TokenCountRecord {
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

impl From<TokenCountRecord> for forge_domain::TokenCount {
    fn from(record: TokenCountRecord) -> Self {
        match record {
            TokenCountRecord::Actual(n) => Self::Actual(n),
            TokenCountRecord::Approx(n) => Self::Approx(n),
        }
    }
}

/// Repository-specific representation of Usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct UsageRecord {
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

impl From<UsageRecord> for forge_domain::Usage {
    fn from(record: UsageRecord) -> Self {
        forge_domain::Usage {
            prompt_tokens: record.prompt_tokens.into(),
            completion_tokens: record.completion_tokens.into(),
            total_tokens: record.total_tokens.into(),
            cached_tokens: record.cached_tokens.into(),
            cost: record.cost,
        }
    }
}

/// Repository-specific representation of EventValue
/// Stored as raw JSON to avoid coupling with UserPrompt and UserCommand
/// structures
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct EventValueRecord(serde_json::Value);

impl From<&forge_domain::EventValue> for EventValueRecord {
    fn from(event: &forge_domain::EventValue) -> Self {
        Self(serde_json::to_value(event).unwrap_or_default())
    }
}

impl TryFrom<EventValueRecord> for forge_domain::EventValue {
    type Error = anyhow::Error;

    fn try_from(record: EventValueRecord) -> anyhow::Result<Self> {
        Ok(serde_json::from_value(record.0)?)
    }
}

/// Repository-specific representation of Role
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RoleRecord {
    System,
    User,
    Assistant,
}

impl From<&forge_domain::Role> for RoleRecord {
    fn from(role: &forge_domain::Role) -> Self {
        match role {
            forge_domain::Role::System => Self::System,
            forge_domain::Role::User => Self::User,
            forge_domain::Role::Assistant => Self::Assistant,
        }
    }
}

impl From<RoleRecord> for forge_domain::Role {
    fn from(record: RoleRecord) -> Self {
        match record {
            RoleRecord::System => Self::System,
            RoleRecord::User => Self::User,
            RoleRecord::Assistant => Self::Assistant,
        }
    }
}

/// Repository-specific representation of TextMessage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TextMessageRecord {
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

/// Helper function for serde to skip serializing false boolean values
fn is_false(value: &bool) -> bool {
    !value
}

impl From<&forge_domain::TextMessage> for TextMessageRecord {
    fn from(msg: &forge_domain::TextMessage) -> Self {
        Self {
            role: RoleRecord::from(&msg.role),
            content: msg.content.clone(),
            raw_content: msg.raw_content.as_ref().map(EventValueRecord::from),
            tool_calls: msg
                .tool_calls
                .as_ref()
                .map(|calls| calls.iter().map(ToolCallFullRecord::from).collect()),
            thought_signature: msg.thought_signature.clone(),
            model: msg.model.as_ref().map(ModelIdRecord::from),
            reasoning_details: msg
                .reasoning_details
                .as_ref()
                .map(|details| details.iter().map(ReasoningFullRecord::from).collect()),
            droppable: msg.droppable,
        }
    }
}

impl TryFrom<TextMessageRecord> for forge_domain::TextMessage {
    type Error = anyhow::Error;

    fn try_from(record: TextMessageRecord) -> anyhow::Result<Self> {
        Ok(forge_domain::TextMessage {
            role: record.role.into(),
            content: record.content,
            raw_content: record.raw_content.map(TryInto::try_into).transpose()?,
            tool_calls: record
                .tool_calls
                .map(|calls| calls.into_iter().map(Into::into).collect()),
            thought_signature: record.thought_signature,
            model: record.model.map(Into::into),
            reasoning_details: record
                .reasoning_details
                .map(|details| details.into_iter().map(Into::into).collect()),
            droppable: record.droppable,
            phase: None,
        })
    }
}

/// Repository-specific representation of ToolValue
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum ToolValueRecord {
    Text(String),
    AI {
        value: String,
        conversation_id: String,
    },
    Image(ImageRecord),
    Empty,
    // Legacy variants for backward compatibility with old conversations
    // These were removed from the domain model but may exist in stored data
    /// Legacy: Markdown-formatted text (now converted to Text)
    Markdown(String),
    /// Legacy: File diff information (now converted to Text summary)
    FileDiff(FileDiffRecord),
    /// Legacy: Paired value for LLM/display (now flattened to first element)
    Pair(Box<ToolValueRecord>, Box<ToolValueRecord>),
}

/// Legacy record for FileDiff - kept for backward compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FileDiffRecord {
    pub path: String,
    pub old_text: Option<String>,
    pub new_text: String,
}

impl From<&forge_domain::ToolValue> for ToolValueRecord {
    fn from(value: &forge_domain::ToolValue) -> Self {
        match value {
            forge_domain::ToolValue::Text(text) => Self::Text(text.clone()),
            forge_domain::ToolValue::AI { value, conversation_id } => Self::AI {
                value: value.clone(),
                conversation_id: conversation_id.into_string(),
            },
            forge_domain::ToolValue::Image(img) => Self::Image(ImageRecord::from(img)),
            forge_domain::ToolValue::Empty => Self::Empty,
        }
    }
}

impl TryFrom<ToolValueRecord> for forge_domain::ToolValue {
    type Error = anyhow::Error;

    fn try_from(record: ToolValueRecord) -> anyhow::Result<Self> {
        Ok(match record {
            ToolValueRecord::Text(text) => Self::Text(text),
            ToolValueRecord::AI { value, conversation_id } => Self::AI {
                value,
                conversation_id: ConversationId::parse(conversation_id)?,
            },
            ToolValueRecord::Image(img) => Self::Image(img.into()),
            ToolValueRecord::Empty => Self::Empty,
            // Legacy variant migrations
            ToolValueRecord::Markdown(md) => Self::Text(md),
            ToolValueRecord::FileDiff(diff) => {
                // Convert FileDiff to a text summary
                Self::Text(format!("[File diff: {}]", diff.path))
            }
            ToolValueRecord::Pair(first, _second) => {
                // Take the first value (LLM-facing content) and convert it
                (*first).try_into()?
            }
        })
    }
}

/// Repository-specific representation of ToolOutput
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolOutputRecord {
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

impl TryFrom<ToolOutputRecord> for forge_domain::ToolOutput {
    type Error = anyhow::Error;

    fn try_from(record: ToolOutputRecord) -> anyhow::Result<Self> {
        let values: Result<Vec<_>, _> = record.values.into_iter().map(TryInto::try_into).collect();
        Ok(forge_domain::ToolOutput { is_error: record.is_error, values: values? })
    }
}

/// Repository-specific representation of ToolResult
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ToolResultRecord {
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

impl TryFrom<ToolResultRecord> for forge_domain::ToolResult {
    type Error = anyhow::Error;

    fn try_from(record: ToolResultRecord) -> anyhow::Result<Self> {
        Ok(forge_domain::ToolResult {
            name: record.name.into(),
            call_id: record.call_id.map(Into::into),
            output: record.output.try_into()?,
        })
    }
}

/// Repository-specific representation of ContextMessageValue
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ContextMessageValueRecord {
    Text(TextMessageRecord),
    Tool(ToolResultRecord),
    Image(ImageRecord),
}

impl From<&forge_domain::ContextMessage> for ContextMessageValueRecord {
    fn from(value: &forge_domain::ContextMessage) -> Self {
        match value {
            forge_domain::ContextMessage::Text(msg) => Self::Text(TextMessageRecord::from(msg)),
            forge_domain::ContextMessage::Tool(result) => {
                Self::Tool(ToolResultRecord::from(result))
            }
            forge_domain::ContextMessage::Image(img) => Self::Image(ImageRecord::from(img)),
        }
    }
}

impl TryFrom<ContextMessageValueRecord> for forge_domain::ContextMessage {
    type Error = anyhow::Error;

    fn try_from(record: ContextMessageValueRecord) -> anyhow::Result<Self> {
        Ok(match record {
            ContextMessageValueRecord::Text(msg) => Self::Text(msg.try_into()?),
            ContextMessageValueRecord::Tool(result) => Self::Tool(result.try_into()?),
            ContextMessageValueRecord::Image(img) => Self::Image(img.into()),
        })
    }
}

/// Repository-specific representation of ContextMessage
#[derive(Debug, Clone, Serialize)]
pub(super) struct ContextMessageRecord {
    message: ContextMessageValueRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<UsageRecord>,
}

// TODO: Move this deserialization logic into Conversation repo
impl<'de> Deserialize<'de> for ContextMessageRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ContextMessageParser {
            // Try new format first (with message field)
            Wrapper {
                message: ContextMessageValueRecord,
                usage: Option<UsageRecord>,
            },
            // Fall back to old format (direct ContextMessage)
            Direct(ContextMessageValueRecord),
        }

        match ContextMessageParser::deserialize(deserializer)? {
            ContextMessageParser::Wrapper { message, usage } => {
                Ok(ContextMessageRecord { message, usage })
            }
            ContextMessageParser::Direct(message) => {
                Ok(ContextMessageRecord { message, usage: None })
            }
        }
    }
}

impl From<&forge_domain::MessageEntry> for ContextMessageRecord {
    fn from(msg: &forge_domain::MessageEntry) -> Self {
        Self {
            message: ContextMessageValueRecord::from(&msg.message),
            usage: msg.usage.as_ref().map(UsageRecord::from),
        }
    }
}

impl TryFrom<ContextMessageRecord> for forge_domain::MessageEntry {
    type Error = anyhow::Error;

    fn try_from(record: ContextMessageRecord) -> anyhow::Result<Self> {
        Ok(forge_domain::MessageEntry {
            message: record.message.try_into()?,
            usage: record.usage.map(Into::into),
        })
    }
}

/// Repository-specific representation of ToolDefinition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ToolDefinitionRecord {
    name: ToolNameRecord,
    description: String,
    input_schema: serde_json::Value,
}

impl From<&forge_domain::ToolDefinition> for ToolDefinitionRecord {
    fn from(def: &forge_domain::ToolDefinition) -> Self {
        Self {
            name: ToolNameRecord::from(&def.name),
            description: def.description.clone(),
            input_schema: serde_json::to_value(&def.input_schema).unwrap_or_default(),
        }
    }
}

impl TryFrom<ToolDefinitionRecord> for forge_domain::ToolDefinition {
    type Error = anyhow::Error;

    fn try_from(record: ToolDefinitionRecord) -> anyhow::Result<Self> {
        Ok(forge_domain::ToolDefinition {
            name: record.name.into(),
            description: record.description,
            input_schema: serde_json::from_value(record.input_schema)?,
        })
    }
}

/// Repository-specific representation of ToolChoice
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) enum ToolChoiceRecord {
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

impl From<ToolChoiceRecord> for forge_domain::ToolChoice {
    fn from(record: ToolChoiceRecord) -> Self {
        match record {
            ToolChoiceRecord::None => Self::None,
            ToolChoiceRecord::Auto => Self::Auto,
            ToolChoiceRecord::Required => Self::Required,
            ToolChoiceRecord::Call(name) => Self::Call(name.into()),
        }
    }
}

/// Repository-specific representation of Effort
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum EffortRecord {
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

impl From<EffortRecord> for forge_domain::Effort {
    fn from(record: EffortRecord) -> Self {
        match record {
            EffortRecord::None => Self::None,
            EffortRecord::Minimal => Self::Minimal,
            EffortRecord::Low => Self::Low,
            EffortRecord::Medium => Self::Medium,
            EffortRecord::High => Self::High,
            EffortRecord::XHigh => Self::XHigh,
            EffortRecord::Max => Self::Max,
        }
    }
}

/// Repository-specific representation of ReasoningConfig
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ReasoningConfigRecord {
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

impl From<ReasoningConfigRecord> for forge_domain::ReasoningConfig {
    fn from(record: ReasoningConfigRecord) -> Self {
        forge_domain::ReasoningConfig {
            effort: record.effort.map(Into::into),
            max_tokens: record.max_tokens,
            exclude: record.exclude,
            enabled: record.enabled,
        }
    }
}

/// Repository-specific representation of Context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ContextRecord {
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
            temperature: context.temperature.map(|t| t.value()),
            top_p: context.top_p.map(|t| t.value()),
            top_k: context.top_k.map(|t| t.value()),
            reasoning: context.reasoning.as_ref().map(ReasoningConfigRecord::from),
            stream: context.stream,
        }
    }
}

impl TryFrom<ContextRecord> for Context {
    type Error = anyhow::Error;

    fn try_from(record: ContextRecord) -> anyhow::Result<Self> {
        let conversation_id = record
            .conversation_id
            .map(ConversationId::parse)
            .transpose()?;

        tracing::debug!(
            "Deserializing context for conversation: {:?}",
            conversation_id
        );

        // Convert messages from repository records to domain types
        let messages: Result<Vec<_>, _> = record
            .messages
            .into_iter()
            .enumerate()
            .map(|(idx, v)| {
                v.try_into().map_err(|e: anyhow::Error| {
                    eprintln!(
                        "Failed to deserialize message {} for conversation {:?}: {}",
                        idx, conversation_id, e
                    );
                    e
                })
            })
            .collect();

        let tools: Result<Vec<_>, _> = record.tools.into_iter().map(TryInto::try_into).collect();

        Ok(Context {
            conversation_id,
            initiator: record.initiator,
            messages: messages?,
            tools: tools?,
            tool_choice: record.tool_choice.map(Into::into),
            max_tokens: record.max_tokens,
            temperature: record
                .temperature
                .map(forge_domain::Temperature::new_unchecked),
            top_p: record.top_p.map(forge_domain::TopP::new_unchecked),
            top_k: record.top_k.map(forge_domain::TopK::new_unchecked),
            reasoning: record.reasoning.map(Into::into),
            stream: record.stream,
            response_format: None,
        })
    }
}

/// Repository-specific representation of FileOperation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FileChangeMetricsRecord {
    lines_added: u64,
    lines_removed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<forge_domain::ToolKind>,
}

impl From<&forge_domain::FileOperation> for FileChangeMetricsRecord {
    fn from(metrics: &forge_domain::FileOperation) -> Self {
        Self {
            lines_added: metrics.lines_added,
            lines_removed: metrics.lines_removed,
            content_hash: metrics.content_hash.clone(),
            tool: Some(metrics.tool),
        }
    }
}

impl From<FileChangeMetricsRecord> for forge_domain::FileOperation {
    fn from(record: FileChangeMetricsRecord) -> Self {
        // Use Write as default tool for old records without tool field
        let tool = record.tool.unwrap_or(forge_domain::ToolKind::Write);
        Self::new(tool)
            .lines_added(record.lines_added)
            .lines_removed(record.lines_removed)
            .content_hash(record.content_hash)
    }
}

/// Represents either a single file operation or array (for backward
/// compatibility)
#[derive(Debug, Clone, Serialize, Deserialize, derive_more::From)]
#[serde(untagged)]
pub(super) enum FileOperationOrArray {
    Single(FileChangeMetricsRecord),
    Array(Vec<FileChangeMetricsRecord>),
}

/// Repository-specific representation of Metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MetricsRecord {
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    files_changed: std::collections::HashMap<String, FileOperationOrArray>,
    #[serde(default, skip_serializing_if = "std::collections::HashSet::is_empty")]
    files_accessed: std::collections::HashSet<String>,
}

impl From<&forge_domain::Metrics> for MetricsRecord {
    fn from(metrics: &forge_domain::Metrics) -> Self {
        Self {
            started_at: metrics.started_at,
            files_changed: metrics
                .file_operations
                .iter()
                .map(|(path, file_metrics)| {
                    (
                        path.clone(),
                        FileOperationOrArray::Single(file_metrics.into()),
                    )
                })
                .collect(),
            files_accessed: metrics.files_accessed.clone(),
        }
    }
}

impl From<MetricsRecord> for forge_domain::Metrics {
    fn from(record: MetricsRecord) -> Self {
        let file_operations: std::collections::HashMap<String, forge_domain::FileOperation> =
            record
                .files_changed
                .into_iter()
                .filter_map(|(path, file_record)| {
                    let operation = match file_record {
                        // If it's an array, take the last operation (most recent)
                        FileOperationOrArray::Array(mut arr) if !arr.is_empty() => {
                            arr.pop().unwrap().into()
                        }
                        // If it's a single object, use it directly
                        FileOperationOrArray::Single(record) => record.into(),
                        // If it's an empty array, skip this file
                        FileOperationOrArray::Array(_) => return None,
                    };
                    Some((path, operation))
                })
                .collect();

        // Use persisted files_accessed if available, otherwise build from Read
        // operations
        let files_accessed = if record.files_accessed.is_empty() {
            // Legacy data: build from Read operations
            file_operations
                .iter()
                .filter(|(_, op)| op.tool == forge_domain::ToolKind::Read)
                .map(|(path, _)| path.clone())
                .collect()
        } else {
            // Use persisted set
            record.files_accessed
        };

        Self {
            started_at: record.started_at,
            file_operations,
            files_accessed,
            todos: Vec::new(),
        }
    }
}

/// Database model for conversations table
#[derive(Debug, diesel::Queryable, diesel::Selectable, diesel::Insertable, diesel::AsChangeset)]
#[diesel(table_name = crate::database::schema::conversations)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub(super) struct ConversationRecord {
    pub conversation_id: String,
    pub title: Option<String>,
    pub workspace_id: i64,
    pub context: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: Option<chrono::NaiveDateTime>,
    pub metrics: Option<String>,
}

impl ConversationRecord {
    /// Creates a new ConversationRecord from a Conversation domain object
    pub fn new(
        conversation: forge_domain::Conversation,
        workspace_id: forge_domain::WorkspaceHash,
    ) -> Self {
        let context = conversation
            .context
            .as_ref()
            .filter(|ctx| !ctx.messages.is_empty() || ctx.initiator.is_some())
            .map(ContextRecord::from)
            .and_then(|ctx_record| serde_json::to_string(&ctx_record).ok());
        let updated_at = context.as_ref().map(|_| chrono::Utc::now().naive_utc());
        let metrics_record = MetricsRecord::from(&conversation.metrics);
        let metrics = serde_json::to_string(&metrics_record).ok();

        Self {
            conversation_id: conversation.id.into_string(),
            title: conversation.title.clone(),
            context,
            created_at: conversation.metadata.created_at.naive_utc(),
            updated_at,
            workspace_id: workspace_id.id() as i64,
            metrics,
        }
    }
}

impl TryFrom<ConversationRecord> for forge_domain::Conversation {
    type Error = anyhow::Error;

    fn try_from(record: ConversationRecord) -> anyhow::Result<Self> {
        let conversation_id = record.conversation_id.clone();
        let id = ConversationId::parse(conversation_id.clone())
            .with_context(|| format!("Failed to parse conversation ID: {}", conversation_id))?;

        let context = if let Some(context_str) = record.context {
            Some(
                serde_json::from_str::<ContextRecord>(&context_str)
                    .with_context(|| {
                        format!(
                            "Failed to deserialize context for conversation {}",
                            conversation_id
                        )
                    })?
                    .try_into()
                    .with_context(|| {
                        format!(
                            "Failed to convert context record to domain type for conversation {}",
                            conversation_id
                        )
                    })?,
            )
        } else {
            None
        };

        // Deserialize metrics using MetricsRecord for compile-time safety
        let metrics = record
            .metrics
            .and_then(|m| serde_json::from_str::<MetricsRecord>(&m).ok())
            .map(forge_domain::Metrics::from)
            .unwrap_or_else(|| {
                forge_domain::Metrics::default().started_at(record.created_at.and_utc())
            });

        Ok(forge_domain::Conversation::new(id)
            .context(context)
            .title(record.title)
            .metrics(metrics)
            .metadata(
                forge_domain::MetaData::new(record.created_at.and_utc())
                    .updated_at(record.updated_at.map(|updated_at| updated_at.and_utc())),
            ))
    }
}
