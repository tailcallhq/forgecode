use std::fmt::Display;
use std::ops::Deref;

use derive_more::derive::{Display, From};
use derive_setters::Setters;
use forge_template::Element;
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::{ToolCallFull, ToolResult};

/// Helper function for serde to skip serializing false boolean values
fn is_false(value: &bool) -> bool {
    !value
}

use crate::temperature::Temperature;
use crate::top_k::TopK;
use crate::top_p::TopP;
use crate::{
    Attachment, AttachmentContent, ConversationId, EventValue, Image, MessagePhase, ModelId,
    ReasoningFull, ToolChoice, ToolDefinition, ToolOutput, ToolValue, Usage,
};

/// Response format for structured output
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Plain text response
    #[default]
    Text,
    /// JSON response with schema
    JsonSchema(Box<schemars::Schema>),
}

/// Represents a message being sent to the LLM provider
/// NOTE: ToolResults message are part of the larger Request object and not part
/// of the message.
#[derive(Clone, Debug, Deserialize, From, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ContextMessage {
    Text(TextMessage),
    Tool(ToolResult),
    Image(Image),
}

/// Creates a filtered version of ToolOutput that excludes base64 images to
/// avoid serializing large image data in the context output
fn filter_base64_images_from_tool_output(output: &ToolOutput) -> ToolOutput {
    let filtered_values: Vec<ToolValue> = output
        .values
        .iter()
        .map(|value| match value {
            ToolValue::Image(image) => {
                // Skip base64 images (URLs that start with "data:")
                if image.url().starts_with("data:") {
                    ToolValue::Text(format!("[base64 image: {}]", image.mime_type()))
                } else {
                    value.clone()
                }
            }
            _ => value.clone(),
        })
        .collect();

    ToolOutput { is_error: output.is_error, values: filtered_values }
}

impl ContextMessage {
    pub fn content(&self) -> Option<&str> {
        match self {
            ContextMessage::Text(text_message) => Some(&text_message.content),
            ContextMessage::Tool(_) => None,
            ContextMessage::Image(_) => None,
        }
    }

    /// Returns the raw content before template rendering (only for User
    /// messages)
    pub fn as_value(&self) -> Option<&EventValue> {
        match self {
            ContextMessage::Text(text_message) => text_message.raw_content.as_ref(),
            ContextMessage::Tool(_) => None,
            ContextMessage::Image(_) => None,
        }
    }

    /// Estimates the number of tokens in a message using character-based
    /// approximation.
    /// ref: https://github.com/openai/codex/blob/main/codex-cli/src/utils/approximate-tokens-used.ts
    pub fn token_count_approx(&self) -> usize {
        let char_count = match self {
            ContextMessage::Text(text_message) => {
                text_message.content.chars().count()
                    + tool_call_content_char_count(text_message)
                    + reasoning_content_char_count(text_message)
            }
            ContextMessage::Tool(tool_result) => tool_result
                .output
                .values
                .iter()
                .map(|result| match result {
                    ToolValue::Text(text) => text.chars().count(),
                    _ => 0,
                })
                .sum(),
            _ => 0,
        };

        char_count.div_ceil(4)
    }

    pub fn to_text(&self) -> String {
        match self {
            ContextMessage::Text(message) => {
                let mut message_element = Element::new("message").attr("role", message.role);

                message_element =
                    message_element.append(Element::new("content").text(&message.content));

                if let Some(tool_calls) = &message.tool_calls {
                    for call in tool_calls {
                        message_element = message_element.append(
                            Element::new("forge_tool_call")
                                .attr("name", &call.name)
                                .cdata(call.arguments.clone().into_string()),
                        );
                    }
                }

                if let Some(thought_signature) = &message.thought_signature {
                    message_element = message_element
                        .append(Element::new("thought_signature").text(thought_signature));
                }

                if let Some(reasoning_details) = &message.reasoning_details {
                    for reasoning_detail in reasoning_details {
                        if let Some(text) = &reasoning_detail.text {
                            message_element =
                                message_element.append(Element::new("reasoning_detail").text(text));
                        }
                    }
                }

                message_element.render()
            }
            ContextMessage::Tool(result) => {
                let filtered_output = filter_base64_images_from_tool_output(&result.output);
                Element::new("message")
                    .attr("role", "tool")
                    .append(
                        Element::new("forge_tool_result")
                            .attr("name", &result.name)
                            .cdata(serde_json::to_string(&filtered_output).unwrap()),
                    )
                    .render()
            }
            ContextMessage::Image(_) => Element::new("image").attr("path", "[base64 URL]").render(),
        }
    }

    pub fn user(content: impl ToString, model: Option<ModelId>) -> Self {
        TextMessage {
            role: Role::User,
            content: content.to_string(),
            raw_content: None,
            tool_calls: None,
            thought_signature: None,
            reasoning_details: None,
            model,
            droppable: false,
            phase: None,
        }
        .into()
    }

    pub fn system(content: impl ToString) -> Self {
        TextMessage {
            role: Role::System,
            content: content.to_string(),
            raw_content: None,
            tool_calls: None,
            thought_signature: None,
            model: None,
            reasoning_details: None,
            droppable: false,
            phase: None,
        }
        .into()
    }

    pub fn assistant(
        content: impl ToString,
        thought_signature: Option<String>,
        reasoning_details: Option<Vec<ReasoningFull>>,
        tool_calls: Option<Vec<ToolCallFull>>,
    ) -> Self {
        let tool_calls = tool_calls.filter(|calls| !calls.is_empty());
        TextMessage {
            role: Role::Assistant,
            content: content.to_string(),
            raw_content: None,
            tool_calls,
            thought_signature,
            reasoning_details,
            model: None,
            droppable: false,
            phase: None,
        }
        .into()
    }

    pub fn tool_result(result: ToolResult) -> Self {
        Self::Tool(result)
    }

    pub fn has_role(&self, role: Role) -> bool {
        match self {
            ContextMessage::Text(message) => message.role == role,
            ContextMessage::Tool(_) => false,
            ContextMessage::Image(_) => Role::User == role,
        }
    }

    pub fn is_droppable(&self) -> bool {
        match self {
            ContextMessage::Text(message) => message.droppable,
            ContextMessage::Tool(_) => false,
            ContextMessage::Image(_) => false,
        }
    }

    pub fn has_tool_result(&self) -> bool {
        match self {
            ContextMessage::Text(_) => false,
            ContextMessage::Tool(_) => true,
            ContextMessage::Image(_) => false,
        }
    }

    pub fn has_tool_call(&self) -> bool {
        match self {
            ContextMessage::Text(message) => message.tool_calls.is_some(),
            ContextMessage::Tool(_) => false,
            ContextMessage::Image(_) => false,
        }
    }

    pub fn has_reasoning_details(&self) -> bool {
        match self {
            ContextMessage::Text(message) => message.reasoning_details.is_some(),
            ContextMessage::Tool(_) => false,
            ContextMessage::Image(_) => false,
        }
    }

    /// Returns the tool result if this message is a Tool variant
    pub fn as_tool_result(&self) -> Option<&ToolResult> {
        match self {
            ContextMessage::Tool(result) => Some(result),
            _ => None,
        }
    }
}

fn tool_call_content_char_count(text_message: &TextMessage) -> usize {
    text_message
        .tool_calls
        .as_ref()
        .map(|tool_calls| {
            tool_calls
                .iter()
                .map(|tc| {
                    tc.arguments.to_owned().into_string().chars().count()
                        + tc.name.as_str().chars().count()
                })
                .sum()
        })
        .unwrap_or(0)
}

fn reasoning_content_char_count(text_message: &TextMessage) -> usize {
    text_message
        .reasoning_details
        .as_ref()
        .map_or(0, |details| {
            details
                .iter()
                .map(|rd| rd.text.as_ref().map_or(0, |text| text.chars().count()))
                .sum::<usize>()
        })
}

//TODO: Rename to TextMessage
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, Setters)]
#[setters(strip_option, into)]
#[serde(rename_all = "snake_case")]
pub struct TextMessage {
    pub role: Role,
    pub content: String,
    /// The raw content before any template rendering (only for User messages)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_content: Option<EventValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallFull>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
    // note: this used to track model used for this message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_details: Option<Vec<ReasoningFull>>,
    /// Indicates whether this message can be dropped during context compaction
    #[serde(default, skip_serializing_if = "is_false")]
    pub droppable: bool,
    /// Phase label for assistant messages (`Commentary` or `FinalAnswer`).
    /// Preserved from OpenAI Responses API and replayed back on subsequent
    /// requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<MessagePhase>,
}

impl TextMessage {
    /// Creates a new TextMessage with the given role and content
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            raw_content: None,
            tool_calls: None,
            thought_signature: None,
            model: None,
            reasoning_details: None,
            droppable: false,
            phase: None,
        }
    }

    pub fn has_role(&self, role: Role) -> bool {
        self.role == role
    }

    pub fn assistant(
        content: impl ToString,
        reasoning_details: Option<Vec<ReasoningFull>>,
        model: Option<ModelId>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.to_string(),
            raw_content: None,
            tool_calls: None,
            thought_signature: None,
            reasoning_details,
            model,
            droppable: false,
            phase: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize, Display)]
pub enum Role {
    System,
    User,
    Assistant,
}
#[derive(Clone, Debug, Serialize, Deserialize, Setters, PartialEq)]
#[setters(into, strip_option)]
pub struct MessageEntry {
    #[serde(flatten)]
    pub message: ContextMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl From<ContextMessage> for MessageEntry {
    fn from(value: ContextMessage) -> Self {
        MessageEntry { message: value, usage: Default::default() }
    }
}

impl Deref for MessageEntry {
    type Target = ContextMessage;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl std::ops::DerefMut for MessageEntry {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.message
    }
}

/// Represents a request being made to the LLM provider. By default the request
/// is created with assuming the model supports use of external tools.
#[derive(Clone, Debug, Deserialize, Serialize, Setters, Default, PartialEq)]
#[setters(into, strip_option)]
pub struct Context {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    /// Indicates who initiated the conversation: "user" or "agent".
    /// Used for GitHub Copilot billing optimization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<MessageEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Temperature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<TopP>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<TopK>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<crate::ReasoningConfig>,
    /// Controls whether responses should be streamed. When `true`, responses
    /// are delivered incrementally as they're generated. When `false`, the
    /// complete response is returned at once. Defaults to `true` if not
    /// specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Response format for structured output (JSON schema)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
}

impl Context {
    pub fn accumulate_usage(&self) -> Option<Usage> {
        self.messages
            .iter()
            .filter_map(|msg| msg.usage.as_ref())
            .cloned()
            .reduce(|a, b| a.accumulate(&b))
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.messages
            .iter()
            .find(|message| message.has_role(Role::System))
            .and_then(|msg| msg.content())
    }

    pub fn add_base64_url(mut self, image: Image) -> Self {
        self.messages.push(ContextMessage::Image(image).into());
        self
    }

    pub fn add_tool(mut self, tool: impl Into<ToolDefinition>) -> Self {
        let tool: ToolDefinition = tool.into();
        self.tools.push(tool);
        self
    }

    pub fn add_message(self, content: impl Into<ContextMessage>) -> Self {
        self.add_entry(content.into())
    }

    pub fn add_entry(mut self, content: impl Into<MessageEntry>) -> Self {
        let content = content.into();
        self.messages.push(content);

        self
    }

    pub fn add_attachments(self, attachments: Vec<Attachment>, model_id: Option<ModelId>) -> Self {
        attachments.into_iter().fold(self, |ctx, attachment| {
            ctx.add_message(match attachment.content {
                AttachmentContent::Image(image) => ContextMessage::Image(image),
                AttachmentContent::FileContent { content, info } => {
                    let elm = Element::new("file_content")
                        .attr("path", attachment.path)
                        .attr("start_line", info.start_line)
                        .attr("end_line", info.end_line)
                        .attr("total_lines", info.total_lines)
                        .cdata(content);

                    let mut message = TextMessage::new(Role::User, elm.to_string()).droppable(true);

                    if let Some(model) = model_id.clone() {
                        message = message.model(model);
                    }

                    message.into()
                }
                AttachmentContent::DirectoryListing { entries } => {
                    let elm = Element::new("directory_listing")
                        .attr("path", attachment.path)
                        .append(entries.into_iter().map(|entry| {
                            let tag_name = if entry.is_dir { "dir" } else { "file" };
                            Element::new(tag_name).text(entry.path)
                        }));

                    let mut message = TextMessage::new(Role::User, elm.to_string()).droppable(true);

                    if let Some(model) = model_id.clone() {
                        message = message.model(model);
                    }

                    message.into()
                }
            })
        })
    }

    pub fn add_tool_results(mut self, results: Vec<ToolResult>) -> Self {
        if !results.is_empty() {
            debug!(results = ?results, "Adding tool results to context");
            self.messages.extend(
                results
                    .into_iter()
                    .map(ContextMessage::tool_result)
                    .map(MessageEntry::from),
            );
        }

        self
    }

    /// Updates the set system message
    pub fn set_system_messages<S: Into<String>>(mut self, content: Vec<S>) -> Self {
        if self.messages.is_empty() {
            for message in content {
                self.messages
                    .push(ContextMessage::system(message.into()).into());
            }
            self
        } else {
            // drop all the system messages;
            self.messages.retain(|m| !m.has_role(Role::System));
            // add the system message at the beginning.
            for message in content.into_iter().rev() {
                self.messages
                    .insert(0, ContextMessage::system(message.into()).into());
            }
            self
        }
    }

    /// Converts the context to textual format
    pub fn to_text(&self) -> String {
        let mut lines = String::new();

        for message in self.messages.iter() {
            lines.push_str(&message.to_text());
        }

        format!("<chat_history>{lines}</chat_history>")
    }

    /// Will append a message to the context. This method always assumes tools
    /// are supported and uses the appropriate format. For models that don't
    /// support tools, use the TransformToolCalls transformer to convert the
    /// context afterward.
    #[allow(clippy::too_many_arguments)]
    pub fn append_message(
        self,
        content: impl ToString,
        thought_signature: Option<String>,
        reasoning: Option<String>,
        reasoning_details: Option<Vec<ReasoningFull>>,
        usage: Usage,
        tool_records: Vec<(ToolCallFull, ToolResult)>,
        phase: Option<MessagePhase>,
    ) -> Self {
        // Convert flat reasoning string to reasoning_details only when no structured
        // reasoning_details are present. When reasoning_details already exists it
        // already contains the text (with its cryptographic signature), so adding
        // another entry from the raw `reasoning` string would produce a duplicate
        // thinking block with a null signature, which Anthropic rejects.
        let merged_reasoning_details = match (reasoning, reasoning_details) {
            (_, Some(details)) => Some(details),
            (Some(reasoning_text), None) => Some(vec![ReasoningFull {
                text: Some(reasoning_text),
                type_of: Some("reasoning.text".to_string()),
                ..Default::default()
            }]),
            (None, None) => None,
        };

        // Adding tool calls
        let mut message: MessageEntry = ContextMessage::assistant(
            content,
            thought_signature,
            merged_reasoning_details,
            Some(
                tool_records
                    .iter()
                    .map(|record| record.0.clone())
                    .collect::<Vec<_>>(),
            ),
        )
        .into();

        // Set phase on the assistant TextMessage if provided
        if let ContextMessage::Text(ref mut text_msg) = message.message {
            text_msg.phase = phase;
        }

        let tool_results = tool_records
            .iter()
            .map(|record| record.1.clone())
            .collect::<Vec<_>>();

        self.add_entry(message.usage(usage))
            .add_tool_results(tool_results)
    }

    /// Returns the token count for context
    pub fn token_count(&self) -> TokenCount {
        let actual = self
            .messages
            .last()
            .as_ref()
            .and_then(|u| u.usage)
            .map(|u| u.total_tokens)
            .unwrap_or_default();

        match actual {
            TokenCount::Actual(actual) if actual > 0 => TokenCount::Actual(actual),
            _ => TokenCount::Approx(self.token_count_approx()),
        }
    }

    pub fn token_count_approx(&self) -> usize {
        self.messages
            .iter()
            .map(|m| m.token_count_approx())
            .sum::<usize>()
    }

    /// Checks if reasoning is enabled by user or not.
    pub fn is_reasoning_supported(&self) -> bool {
        self.reasoning.as_ref().is_some_and(|reasoning| {
            // `Effort::None` is a strong opt-out that wins over `enabled` and
            // `max_tokens`.
            if matches!(reasoning.effort, Some(crate::Effort::None)) {
                return false;
            }

            // When enabled parameter is defined then return it's value directly.
            if reasoning.enabled.is_some() {
                return reasoning.enabled.unwrap_or_default();
            }

            // If not defined (None), check other parameters
            reasoning.effort.is_some() || reasoning.max_tokens.is_some_and(|token| token > 0)
        })
    }

    /// Returns a vector of user messages, selecting the first message from
    /// each consecutive sequence of user messages.
    pub fn first_user_messages(&self) -> Vec<&ContextMessage> {
        if self.messages.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        let mut is_user = false;

        for msg in &self.messages {
            if msg.has_role(Role::User) {
                // Only add the first message of each consecutive user sequence
                if !is_user {
                    result.push(&**msg);
                    is_user = true;
                }
            } else {
                is_user = false;
            }
        }

        result
    }

    /// Returns the total number of messages in the context
    pub fn total_messages(&self) -> usize {
        self.messages.len()
    }

    /// Returns the count of user messages in the context
    pub fn user_message_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|msg| msg.has_role(Role::User))
            .count()
    }

    /// Returns the count of assistant messages in the context
    pub fn assistant_message_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|msg| msg.has_role(Role::Assistant))
            .count()
    }

    /// Returns the total count of tool calls across all messages
    pub fn tool_call_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|msg| msg.has_tool_call())
            .map(|msg| {
                if let ContextMessage::Text(text_msg) = &**msg {
                    text_msg.tool_calls.as_ref().map_or(0, |calls| calls.len())
                } else {
                    0
                }
            })
            .sum()
    }

    /// Checks if the model has changed from the previous assistant message.
    /// Returns true if the previous assistant message has a different model
    /// than the provided current_model, or if there is no previous
    /// assistant message with a model.
    ///
    /// This is used to determine whether to apply reasoning normalization - we
    /// only want to strip reasoning when switching models, not when
    /// continuing with the same model.
    pub fn has_model_changed(&self, current_model: &ModelId) -> bool {
        // Find the last assistant message with a model field
        let last_assistant_model = self.messages.iter().rev().find_map(|msg| {
            if let ContextMessage::Text(text_msg) = &**msg
                && text_msg.has_role(Role::Assistant)
            {
                return text_msg.model.as_ref();
            }
            None
        });

        // If there's no previous assistant model, consider it as changed
        // If there is a previous model, check if it differs from current
        match last_assistant_model {
            None => true,
            Some(prev_model) => prev_model != current_model,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TokenCount {
    Actual(usize),
    Approx(usize),
}

impl Display for TokenCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenCount::Actual(count) => write!(f, "{count}"),
            TokenCount::Approx(count) => write!(f, "~{count}"),
        }
    }
}

impl std::ops::Add for TokenCount {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        match (self, other) {
            (TokenCount::Actual(a), TokenCount::Actual(b)) => TokenCount::Actual(a + b),
            (TokenCount::Approx(a), TokenCount::Approx(b)) => TokenCount::Approx(a + b),
            (TokenCount::Actual(a), TokenCount::Approx(b)) => TokenCount::Approx(a + b),
            (TokenCount::Approx(a), TokenCount::Actual(b)) => TokenCount::Approx(a + b),
        }
    }
}

impl Default for TokenCount {
    fn default() -> Self {
        TokenCount::Actual(0)
    }
}

impl TokenCount {
    /// Returns the larger of two TokenCount values by their inner count.
    /// If both are `Actual`, the result is `Actual`. If either is `Approx`,
    /// the result is `Approx`.
    pub fn max(self, other: TokenCount) -> TokenCount {
        use TokenCount::*;
        match (self, other) {
            (Actual(a), Actual(b)) => Actual(a.max(b)),
            (Actual(a), Approx(b)) => Approx(a.max(b)),
            (Approx(a), Actual(b)) => Approx(a.max(b)),
            (Approx(a), Approx(b)) => Approx(a.max(b)),
        }
    }
}

impl Deref for TokenCount {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        match self {
            TokenCount::Actual(i) => i,
            TokenCount::Approx(i) => i,
        }
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_yaml_snapshot;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::transformer::Transformer;
    use crate::{DirectoryEntry, FileInfo, estimate_token_count};

    #[test]
    fn test_override_system_message() {
        let request = Context::default()
            .add_message(ContextMessage::system("Initial system message"))
            .set_system_messages(vec!["Updated system message"]);

        assert_eq!(
            request.messages[0],
            ContextMessage::system("Updated system message").into(),
        );
    }

    #[test]
    fn test_set_system_message() {
        let request = Context::default().set_system_messages(vec!["A system message"]);

        assert_eq!(
            request.messages[0],
            ContextMessage::system("A system message").into(),
        );
    }

    #[test]
    fn test_insert_system_message() {
        let model = ModelId::new("test-model");
        let request = Context::default()
            .add_message(ContextMessage::user("Do something", Some(model)))
            .set_system_messages(vec!["A system message"]);

        assert_eq!(
            request.messages[0],
            ContextMessage::system("A system message").into(),
        );
    }

    #[test]
    fn test_estimate_token_count() {
        // Create a context with some messages
        let model = ModelId::new("test-model");
        let context = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_message(ContextMessage::user("User message", model.into()))
            .add_message(ContextMessage::assistant(
                "Assistant message",
                None,
                None,
                None,
            ));

        // Get the token count
        let token_count = estimate_token_count(context.to_text().len());

        // Validate the token count is reasonable
        // The exact value will depend on the implementation of estimate_token_count
        assert!(token_count > 0, "Token count should be greater than 0");
    }

    #[test]
    fn test_update_image_tool_calls_empty_context() {
        let fixture = Context::default();
        let mut transformer = crate::transformer::ImageHandling::new();
        let actual = transformer.transform(fixture);

        assert_yaml_snapshot!(actual);
    }

    #[test]
    fn test_update_image_tool_calls_no_tool_results() {
        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_message(ContextMessage::user("User message", None))
            .add_message(ContextMessage::assistant(
                "Assistant message",
                None,
                None,
                None,
            ));
        let mut transformer = crate::transformer::ImageHandling::new();
        let actual = transformer.transform(fixture);

        assert_yaml_snapshot!(actual);
    }

    #[test]
    fn test_update_image_tool_calls_tool_results_no_images() {
        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_tool_results(vec![
                ToolResult {
                    name: crate::ToolName::new("text_tool"),
                    call_id: Some(crate::ToolCallId::new("call1")),
                    output: crate::ToolOutput::text("Text output".to_string()),
                },
                ToolResult {
                    name: crate::ToolName::new("empty_tool"),
                    call_id: Some(crate::ToolCallId::new("call2")),
                    output: crate::ToolOutput {
                        values: vec![crate::ToolValue::Empty],
                        is_error: false,
                    },
                },
            ]);

        let mut transformer = crate::transformer::ImageHandling::new();
        let actual = transformer.transform(fixture);

        assert_yaml_snapshot!(actual);
    }

    #[test]
    fn test_update_image_tool_calls_single_image() {
        let image = Image::new_base64("test123".to_string(), "image/png");
        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_tool_results(vec![ToolResult {
                name: crate::ToolName::new("image_tool"),
                call_id: Some(crate::ToolCallId::new("call1")),
                output: crate::ToolOutput::image(image),
            }]);

        let mut transformer = crate::transformer::ImageHandling::new();
        let actual = transformer.transform(fixture);

        assert_yaml_snapshot!(actual);
    }

    #[test]
    fn test_update_image_tool_calls_multiple_images_single_tool_result() {
        let image1 = Image::new_base64("test123".to_string(), "image/png");
        let image2 = Image::new_base64("test456".to_string(), "image/jpeg");
        let fixture = Context::default().add_tool_results(vec![ToolResult {
            name: crate::ToolName::new("multi_image_tool"),
            call_id: Some(crate::ToolCallId::new("call1")),
            output: crate::ToolOutput {
                values: vec![
                    crate::ToolValue::Text("First text".to_string()),
                    crate::ToolValue::Image(image1),
                    crate::ToolValue::Text("Second text".to_string()),
                    crate::ToolValue::Image(image2),
                ],
                is_error: false,
            },
        }]);

        let mut transformer = crate::transformer::ImageHandling::new();
        let actual = transformer.transform(fixture);

        assert_yaml_snapshot!(actual);
    }

    #[test]
    fn test_update_image_tool_calls_multiple_tool_results_with_images() {
        let image1 = Image::new_base64("test123".to_string(), "image/png");
        let image2 = Image::new_base64("test456".to_string(), "image/jpeg");
        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_tool_results(vec![
                ToolResult {
                    name: crate::ToolName::new("text_tool"),
                    call_id: Some(crate::ToolCallId::new("call1")),
                    output: crate::ToolOutput::text("Text output".to_string()),
                },
                ToolResult {
                    name: crate::ToolName::new("image_tool1"),
                    call_id: Some(crate::ToolCallId::new("call2")),
                    output: crate::ToolOutput::image(image1),
                },
                ToolResult {
                    name: crate::ToolName::new("image_tool2"),
                    call_id: Some(crate::ToolCallId::new("call3")),
                    output: crate::ToolOutput::image(image2),
                },
            ]);

        let mut transformer = crate::transformer::ImageHandling::new();
        let actual = transformer.transform(fixture);

        assert_yaml_snapshot!(actual);
    }

    #[test]
    fn test_update_image_tool_calls_mixed_content_with_images() {
        let image = Image::new_base64("test123".to_string(), "image/png");
        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_message(ContextMessage::user("User question", None))
            .add_message(ContextMessage::assistant(
                "Assistant response",
                None,
                None,
                None,
            ))
            .add_tool_results(vec![ToolResult {
                name: crate::ToolName::new("mixed_tool"),
                call_id: Some(crate::ToolCallId::new("call1")),
                output: crate::ToolOutput {
                    values: vec![
                        crate::ToolValue::Text("Before image".to_string()),
                        crate::ToolValue::Image(image),
                        crate::ToolValue::Text("After image".to_string()),
                        crate::ToolValue::Empty,
                    ],
                    is_error: false,
                },
            }]);

        let mut transformer = crate::transformer::ImageHandling::new();
        let actual = transformer.transform(fixture);

        assert_yaml_snapshot!(actual);
    }

    #[test]
    fn test_update_image_tool_calls_preserves_error_flag() {
        let image = Image::new_base64("test123".to_string(), "image/png");
        let fixture = Context::default().add_tool_results(vec![ToolResult {
            name: crate::ToolName::new("error_tool"),
            call_id: Some(crate::ToolCallId::new("call1")),
            output: crate::ToolOutput {
                values: vec![crate::ToolValue::Image(image)],
                is_error: true,
            },
        }]);

        let mut transformer = crate::transformer::ImageHandling::new();
        let actual = transformer.transform(fixture);

        assert_yaml_snapshot!(actual);
    }

    #[test]
    fn test_context_should_return_max_token_count() {
        let fixture = Context::default();
        let actual = fixture.token_count();
        let expected = TokenCount::Approx(0); // Empty context has no tokens
        assert_eq!(actual, expected);

        // case 2: context with usage - since total_tokens present return that.
        let usage = Usage { total_tokens: TokenCount::Actual(100), ..Default::default() };
        let mut wrapper = MessageEntry::from(ContextMessage::user("Hello", None));
        wrapper.usage = Some(usage);
        let fixture = Context::default().messages(vec![wrapper]);
        assert_eq!(fixture.token_count(), TokenCount::Actual(100));

        // case 3: context with usage - since total_tokens present return that.
        let usage = Usage { total_tokens: TokenCount::Actual(80), ..Default::default() };
        let mut wrapper = MessageEntry::from(ContextMessage::user("Hello", None));
        wrapper.usage = Some(usage);
        let fixture = Context::default().messages(vec![wrapper]);
        assert_eq!(fixture.token_count(), TokenCount::Actual(80));

        // case 4: context with messages - since total_tokens are not present return
        // estimate
        let fixture = Context::default()
            .add_message(ContextMessage::user("Hello", None))
            .add_message(ContextMessage::assistant("Hi there!", None, None, None))
            .add_message(ContextMessage::assistant(
                "How can I help you?",
                None,
                None,
                None,
            ))
            .add_message(ContextMessage::user("I'm looking for a restaurant.", None));
        assert_eq!(fixture.token_count(), TokenCount::Approx(18));
    }

    #[test]
    fn test_context_token_count_uses_last_message_usage() {
        // Setup: Create multiple messages with different usage values
        let first_usage = Usage { total_tokens: TokenCount::Actual(100), ..Default::default() };
        let mut first_message = MessageEntry::from(ContextMessage::user("First message", None));
        first_message.usage = Some(first_usage);

        let second_usage = Usage { total_tokens: TokenCount::Actual(200), ..Default::default() };
        let mut second_message = MessageEntry::from(ContextMessage::assistant(
            "Second message",
            None,
            None,
            None,
        ));
        second_message.usage = Some(second_usage);

        let third_usage = Usage { total_tokens: TokenCount::Actual(300), ..Default::default() };
        let mut third_message = MessageEntry::from(ContextMessage::user("Third message", None));
        third_message.usage = Some(third_usage);

        // Execute: Create context with all three messages
        let fixture =
            Context::default().messages(vec![first_message, second_message, third_message]);

        let actual = fixture.token_count();

        // Expected: Should use the LAST message's usage (300), not the first (100) or
        // second (200)
        let expected = TokenCount::Actual(300);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_is_reasoning_supported_when_enabled() {
        let fixture = Context::default()
            .reasoning(crate::ReasoningConfig { enabled: Some(true), ..Default::default() });

        let actual = fixture.is_reasoning_supported();
        let expected = true;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_is_reasoning_supported_when_effort_set() {
        let fixture = Context::default().reasoning(crate::ReasoningConfig {
            effort: Some(crate::Effort::High),
            ..Default::default()
        });

        let actual = fixture.is_reasoning_supported();
        let expected = true;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_is_reasoning_supported_when_max_tokens_positive() {
        let fixture = Context::default()
            .reasoning(crate::ReasoningConfig { max_tokens: Some(1024), ..Default::default() });

        let actual = fixture.is_reasoning_supported();
        let expected = true;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_is_reasoning_not_supported_when_max_tokens_zero() {
        let fixture = Context::default()
            .reasoning(crate::ReasoningConfig { max_tokens: Some(0), ..Default::default() });

        let actual = fixture.is_reasoning_supported();
        let expected = false;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_is_reasoning_not_supported_when_disabled() {
        let fixture = Context::default()
            .reasoning(crate::ReasoningConfig { enabled: Some(false), ..Default::default() });

        let actual = fixture.is_reasoning_supported();
        let expected = false;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_is_reasoning_not_supported_when_no_config() {
        let fixture = Context::default();

        let actual = fixture.is_reasoning_supported();
        let expected = false;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_is_reasoning_not_supported_when_explicitly_disabled() {
        let fixture = Context::default().reasoning(crate::ReasoningConfig {
            enabled: Some(false),
            effort: Some(crate::Effort::High), /* Should be ignored when
                                                * explicitly disabled */
            ..Default::default()
        });

        let actual = fixture.is_reasoning_supported();
        let expected = false;

        assert_eq!(
            actual, expected,
            "Should not be supported when explicitly disabled, even with effort set"
        );
    }

    #[test]
    fn test_context_is_reasoning_not_supported_when_effort_is_none() {
        // `Effort::None` is documented as "skips the thinking step entirely" and
        // must act as an explicit opt-out regardless of other fields.
        let fixture = Context::default().reasoning(crate::ReasoningConfig {
            effort: Some(crate::Effort::None),
            ..Default::default()
        });

        let actual = fixture.is_reasoning_supported();

        assert!(!actual);
    }

    #[test]
    fn test_context_is_reasoning_not_supported_when_effort_none_overrides_enabled_true() {
        let fixture = Context::default().reasoning(crate::ReasoningConfig {
            enabled: Some(true),
            effort: Some(crate::Effort::None),
            max_tokens: Some(8000),
            ..Default::default()
        });

        let actual = fixture.is_reasoning_supported();

        assert!(
            !actual,
            "Effort::None must win over enabled: true and max_tokens"
        );
    }

    #[test]
    fn test_add_attachments_file_content_is_droppable() {
        let fixture_attachments = vec![Attachment {
            path: "/path/to/file.rs".to_string(),
            content: AttachmentContent::FileContent {
                content: "fn main() {}\n".to_string(),
                info: FileInfo::new(1, 1, 1, "hash".to_string()),
            },
        }];

        let fixture_model = ModelId::new("test-model");
        let actual = Context::default().add_attachments(fixture_attachments, Some(fixture_model));

        // Verify the message was added
        assert_eq!(actual.messages.len(), 1);

        // Verify the message is droppable
        let message = &actual.messages[0];
        assert!(
            message.is_droppable(),
            "File content attachments should be marked as droppable"
        );

        // Verify the message is a User message
        assert!(message.has_role(Role::User));
    }

    #[test]
    fn test_add_attachments_image_is_not_droppable() {
        let fixture_image = Image::new_base64("base64data".to_string(), "image/png");
        let fixture_attachments = vec![Attachment {
            path: "image.png".to_string(),
            content: AttachmentContent::Image(fixture_image),
        }];

        let actual = Context::default().add_attachments(fixture_attachments, None);

        // Verify the message was added
        assert_eq!(actual.messages.len(), 1);

        // Verify the image message is NOT droppable (images use different
        // ContextMessage variant)
        let message = &actual.messages[0];
        assert!(
            !message.is_droppable(),
            "Image attachments should not be marked as droppable"
        );
    }

    #[test]
    fn test_add_attachments_multiple_file_contents_all_droppable() {
        let fixture_attachments = vec![
            Attachment {
                path: "/path/to/file1.rs".to_string(),
                content: AttachmentContent::FileContent {
                    content: "fn foo() {}\n".to_string(),
                    info: FileInfo::new(1, 1, 1, "hash1".to_string()),
                },
            },
            Attachment {
                path: "/path/to/file2.rs".to_string(),
                content: AttachmentContent::FileContent {
                    content: "fn bar() {}\n".to_string(),
                    info: FileInfo::new(1, 1, 1, "hash2".to_string()),
                },
            },
        ];

        let actual = Context::default().add_attachments(fixture_attachments, None);

        // Verify both messages were added
        assert_eq!(actual.messages.len(), 2);

        // Verify all file content messages are droppable
        for message in &actual.messages {
            assert!(
                message.is_droppable(),
                "All file content attachments should be marked as droppable"
            );
        }
    }

    #[test]
    fn test_add_attachments_directory_listing() {
        let fixture_attachments = vec![Attachment {
            path: "/test/mydir".to_string(),
            content: AttachmentContent::DirectoryListing {
                entries: vec![
                    DirectoryEntry { path: "/test/mydir/file1.txt".to_string(), is_dir: false },
                    DirectoryEntry { path: "/test/mydir/file2.rs".to_string(), is_dir: false },
                    DirectoryEntry { path: "/test/mydir/subdir".to_string(), is_dir: true },
                ],
            },
        }];

        let actual = Context::default().add_attachments(fixture_attachments, None);

        // Verify message was added
        assert_eq!(actual.messages.len(), 1);

        // Verify directory listing is formatted correctly as XML
        let message = actual.messages.first().unwrap();
        assert!(
            message.is_droppable(),
            "Directory listing should be marked as droppable"
        );

        let text = message.to_text();
        // The XML is encoded within the message content
        assert!(text.contains("&lt;directory_listing"));
        // Check that files use <file> tag
        assert!(text.contains("&lt;file&gt;"));
        // Check that directories use <dir> tag
        assert!(text.contains("&lt;dir&gt;"));
    }

    #[test]
    fn test_context_message_statistics() {
        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_message(ContextMessage::user("User message 1", None))
            .add_message(ContextMessage::assistant(
                "Assistant response",
                None,
                None,
                None,
            ))
            .add_message(ContextMessage::user("User message 2", None))
            .add_message(ContextMessage::assistant(
                "Assistant with tool",
                None,
                None,
                Some(vec![
                    ToolCallFull {
                        call_id: Some(crate::ToolCallId::new("call1")),
                        name: crate::ToolName::new("tool1"),
                        arguments: serde_json::json!({"arg": "value"}).into(),
                        thought_signature: None,
                    },
                    ToolCallFull {
                        call_id: Some(crate::ToolCallId::new("call2")),
                        name: crate::ToolName::new("tool2"),
                        arguments: serde_json::json!({"arg": "value"}).into(),
                        thought_signature: None,
                    },
                ]),
            ))
            .add_tool_results(vec![
                ToolResult {
                    name: crate::ToolName::new("tool1"),
                    call_id: Some(crate::ToolCallId::new("call1")),
                    output: crate::ToolOutput::text("Result 1".to_string()),
                },
                ToolResult {
                    name: crate::ToolName::new("tool2"),
                    call_id: Some(crate::ToolCallId::new("call2")),
                    output: crate::ToolOutput::text("Result 2".to_string()),
                },
            ]);

        // Test total messages (6 messages: 1 system + 2 user + 2 assistant + 2 tool
        // results)
        assert_eq!(fixture.total_messages(), 7);

        // Test user message count
        assert_eq!(fixture.user_message_count(), 2);

        // Test assistant message count
        assert_eq!(fixture.assistant_message_count(), 2);

        // Test tool call count (2 tool calls in the second assistant message)
        assert_eq!(fixture.tool_call_count(), 2);
    }

    #[test]
    fn test_directory_listing_sorted_dirs_first() {
        // Create entries already sorted (as they would come from attachment service)
        // Directories first, then files, all sorted alphabetically
        let fixture_attachments = vec![Attachment {
            path: "/test/root".to_string(),
            content: AttachmentContent::DirectoryListing {
                entries: vec![
                    DirectoryEntry { path: "apple_dir".to_string(), is_dir: true },
                    DirectoryEntry { path: "berry_dir".to_string(), is_dir: true },
                    DirectoryEntry { path: "zoo_dir".to_string(), is_dir: true },
                    DirectoryEntry { path: "banana.txt".to_string(), is_dir: false },
                    DirectoryEntry { path: "cherry.txt".to_string(), is_dir: false },
                    DirectoryEntry { path: "zebra.txt".to_string(), is_dir: false },
                ],
            },
        }];

        let actual = Context::default().add_attachments(fixture_attachments, None);
        let text = actual.messages.first().unwrap().to_text();

        // Extract the order of entries from the XML
        let dir_entries: Vec<&str> = text
            .split("&lt;")
            .filter(|s| s.starts_with("dir&gt;") || s.starts_with("file&gt;"))
            .collect();

        // Verify directories come first, then files, all sorted alphabetically
        let expected_order = [
            "dir&gt;apple_dir",
            "dir&gt;berry_dir",
            "dir&gt;zoo_dir",
            "file&gt;banana.txt",
            "file&gt;cherry.txt",
            "file&gt;zebra.txt",
        ];

        for (i, expected) in expected_order.iter().enumerate() {
            assert!(
                dir_entries[i].starts_with(expected),
                "Expected entry {} to start with '{}', but got '{}'",
                i,
                expected,
                dir_entries[i]
            );
        }
    }

    #[test]
    fn test_context_message_token_count_approx_user_text() {
        // Fixture: User text message with 40 characters (10 tokens)
        let fixture = ContextMessage::user("This is a test message with content", None);
        let actual = fixture.token_count_approx();
        let expected = 9; // 36 chars / 4 = 9 tokens
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_message_token_count_approx_assistant_text() {
        // Fixture: Assistant text message
        let fixture =
            ContextMessage::assistant("Hello! How can I help you today?", None, None, None);
        let actual = fixture.token_count_approx();
        let expected = 8; // 32 chars / 4 = 8 tokens
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_message_token_count_approx_system() {
        // Fixture: System message should now be counted in token approximation
        let fixture = ContextMessage::system("System instructions here");
        let actual = fixture.token_count_approx();
        let expected = 6; // System messages are now counted in the approximation
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_message_token_count_approx_with_tool_calls() {
        // Fixture: Assistant message with tool calls
        let fixture_tool_calls = vec![
            ToolCallFull {
                call_id: Some(crate::ToolCallId::new("call1")),
                name: crate::ToolName::new("fs_search"),
                arguments: serde_json::json!({"query": "test"}).into(),
                thought_signature: None,
            },
            ToolCallFull {
                call_id: Some(crate::ToolCallId::new("call2")),
                name: crate::ToolName::new("calculate"),
                arguments: serde_json::json!({"expression": "2+2"}).into(),
                thought_signature: None,
            },
        ];
        let fixture =
            ContextMessage::assistant("Let me help", None, None, Some(fixture_tool_calls));
        let actual = fixture.token_count_approx();
        // Content: "Let me help" = 11 chars
        // Tool call 1: "fs_search" (9 chars) + {"query":"test"} (16 chars) = 25 chars
        // Tool call 2: "calculate" (9 chars) + {"expression":"2+2"} (20 chars) = 29
        // chars Total: 11 + 25 + 29 = 65 chars / 4 = 17 tokens
        let expected = 17;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_message_token_count_approx_with_reasoning() {
        // Fixture: Assistant message with reasoning details
        let fixture_reasoning = vec![
            ReasoningFull {
                text: Some("First reasoning step".to_string()),
                ..Default::default()
            },
            ReasoningFull {
                text: Some("Second reasoning step".to_string()),
                ..Default::default()
            },
        ];
        let fixture =
            ContextMessage::assistant("Final answer", None, Some(fixture_reasoning), None);
        let actual = fixture.token_count_approx();
        // Content: "Final answer" = 12 chars = 3 tokens
        // Reasoning 1: "First reasoning step" = 20 chars = 5 tokens
        // Reasoning 2: "Second reasoning step" = 21 chars = 6 tokens
        // Total: 3 + 5 + 6 = 14 tokens
        let expected = 14;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_message_token_count_approx_tool_result_text() {
        // Fixture: Tool result with text output
        let fixture = ContextMessage::tool_result(ToolResult {
            name: crate::ToolName::new("fs_search"),
            call_id: Some(crate::ToolCallId::new("call1")),
            output: crate::ToolOutput::text("Search results: Found 3 items".to_string()),
        });
        let actual = fixture.token_count_approx();
        let expected = 8; // 30 chars / 4 = 8 tokens (rounded up)
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_message_token_count_approx_tool_result_image() {
        // Fixture: Tool result with image (images are not counted)
        let fixture_image = Image::new_base64("base64data".to_string(), "image/png");
        let fixture = ContextMessage::tool_result(ToolResult {
            name: crate::ToolName::new("screenshot"),
            call_id: Some(crate::ToolCallId::new("call1")),
            output: crate::ToolOutput::image(fixture_image),
        });
        let actual = fixture.token_count_approx();
        let expected = 0; // Images are not counted in token approximation
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_message_token_count_approx_image() {
        // Fixture: Image message
        let fixture_image = Image::new_base64("imagedata".to_string(), "image/jpeg");
        let fixture = ContextMessage::Image(fixture_image);
        let actual = fixture.token_count_approx();
        let expected = 0; // Image messages return 0 tokens
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_message_token_count_approx_empty_content() {
        // Fixture: Empty message
        let fixture = ContextMessage::user("", None);
        let actual = fixture.token_count_approx();
        let expected = 0; // 0 chars / 4 = 0 tokens
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_message_token_count_approx_unicode() {
        // Fixture: Message with Unicode characters
        let fixture = ContextMessage::user("Hello 世界 🌍 émojis", None);
        let actual = fixture.token_count_approx();
        // "Hello 世界 🌍 émojis" has 18 Unicode characters
        let expected = 5; // 18 chars / 4 = 5 tokens (rounded up)
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_has_model_changed_returns_true_when_no_previous_messages() {
        let fixture = Context::default();
        let current_model = ModelId::new("gpt-4");

        let actual = fixture.has_model_changed(&current_model);
        let expected = true;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_has_model_changed_returns_true_when_model_differs() {
        let fixture = Context::default()
            .add_message(TextMessage::new(Role::Assistant, "Hello").model(ModelId::new("gpt-3.5")));
        let current_model = ModelId::new("gpt-4");

        let actual = fixture.has_model_changed(&current_model);
        let expected = true;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_has_model_changed_returns_false_when_model_same() {
        let fixture = Context::default()
            .add_message(TextMessage::new(Role::Assistant, "Hello").model(ModelId::new("gpt-4")));
        let current_model = ModelId::new("gpt-4");

        let actual = fixture.has_model_changed(&current_model);
        let expected = false;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_has_model_changed_returns_true_when_previous_has_no_model() {
        let fixture = Context::default().add_message(TextMessage::new(Role::Assistant, "Hello")); // No model set
        let current_model = ModelId::new("gpt-4");

        let actual = fixture.has_model_changed(&current_model);
        let expected = true;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_has_model_changed_checks_last_assistant_message_with_model() {
        let fixture = Context::default()
            .add_message(TextMessage::new(Role::Assistant, "First").model(ModelId::new("gpt-3.5")))
            .add_message(TextMessage::new(Role::User, "Question"))
            .add_message(TextMessage::new(Role::Assistant, "Second").model(ModelId::new("gpt-4")));
        let current_model = ModelId::new("gpt-4");

        let actual = fixture.has_model_changed(&current_model);
        let expected = false; // Last assistant message with model is "gpt-4", same as current

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_has_model_changed_with_multiple_messages_model_changed() {
        let fixture = Context::default()
            .add_message(TextMessage::new(Role::Assistant, "First").model(ModelId::new("gpt-3.5")))
            .add_message(TextMessage::new(Role::User, "Question"))
            .add_message(
                TextMessage::new(Role::Assistant, "Second").model(ModelId::new("claude-3")),
            );
        let current_model = ModelId::new("gpt-4");

        let actual = fixture.has_model_changed(&current_model);
        let expected = true; // Last assistant message with model is "claude-3", different from "gpt-4"

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_has_model_changed_ignores_user_messages() {
        // User messages have model tracking too, but we should only check assistant
        // messages
        let fixture = Context::default()
            .add_message(TextMessage::new(Role::Assistant, "Response").model(ModelId::new("gpt-4")))
            .add_message(TextMessage::new(Role::User, "Question").model(ModelId::new("claude-3")));
        let current_model = ModelId::new("gpt-4");

        let actual = fixture.has_model_changed(&current_model);
        let expected = false; // Last ASSISTANT message is "gpt-4", user message should be ignored

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_has_model_changed_continuing_same_model() {
        // Scenario: model1 -> model2 -> model2 (the second model2 should not drop
        // reasoning)
        let fixture = Context::default()
            .add_message(TextMessage::new(Role::Assistant, "First").model(ModelId::new("model1")))
            .add_message(TextMessage::new(Role::User, "Question"))
            .add_message(TextMessage::new(Role::Assistant, "Second").model(ModelId::new("model2")))
            .add_message(TextMessage::new(Role::User, "Another question"));
        let current_model = ModelId::new("model2");

        let actual = fixture.has_model_changed(&current_model);
        let expected = false; // Last assistant used "model2", same as current

        assert_eq!(actual, expected);
    }

    /// Regression test: when both `reasoning` (raw text) and
    /// `reasoning_details` (structured, with a cryptographic signature) are
    /// present, `append_message` must NOT create a duplicate thinking block
    /// with a null signature.
    ///
    /// The Anthropic API rejects messages where any thinking block carries a
    /// null or missing signature, so the stored `reasoning_details` must
    /// contain exactly the structured entries that were passed in — no
    /// extras.
    #[test]
    fn test_append_message_does_not_duplicate_reasoning_when_details_present() {
        // Fixture: a structured reasoning detail with a valid signature, as would
        // arrive after aggregating an Anthropic streaming response.
        let fixture_details = vec![ReasoningFull {
            text: Some("Let me think about this.".to_string()),
            signature: Some("EpwFvalidSignatureABC123".to_string()),
            type_of: Some("reasoning.text".to_string()),
            format: Some("anthropic-claude-v1".to_string()),
            index: Some(0),
            ..Default::default()
        }];

        // Both reasoning (raw string) and reasoning_details (structured) are provided,
        // mirroring what orch.rs passes after collecting a streamed Anthropic response.
        let fixture = Context::default().add_message(ContextMessage::user("Hello", None));
        let actual = fixture.append_message(
            "Answer",
            None,
            Some("Let me think about this.".to_string()), // raw reasoning string
            Some(fixture_details.clone()),                // structured reasoning_details
            Usage::default(),
            vec![],
            None,
        );

        // Extract the stored reasoning_details from the assistant message.
        let stored = actual
            .messages
            .iter()
            .find_map(|entry| {
                if let ContextMessage::Text(msg) = &**entry
                    && msg.role == Role::Assistant
                {
                    return msg.reasoning_details.as_ref();
                }
                None
            })
            .expect("Assistant message should have reasoning_details");

        // Expected: exactly the one structured entry that was passed in.
        // No duplicate null-signature entry should have been appended.
        let expected = fixture_details;
        assert_eq!(stored, &expected);
    }
}
