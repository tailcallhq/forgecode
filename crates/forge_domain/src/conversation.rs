use std::str::FromStr;

use chrono::{DateTime, Utc};
use derive_more::derive::Display;
use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Context, Error, Metrics, Result, TokenCount};

#[derive(Debug, Default, Display, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct ConversationId(Uuid);

impl Copy for ConversationId {}

impl ConversationId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn into_string(&self) -> String {
        self.0.to_string()
    }

    pub fn parse(value: impl ToString) -> Result<Self> {
        Ok(Self(
            Uuid::parse_str(&value.to_string()).map_err(Error::ConversationId)?,
        ))
    }
}

impl FromStr for ConversationId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

#[derive(Debug, Setters, Serialize, Deserialize, Clone)]
#[setters(into)]
pub struct Conversation {
    pub id: ConversationId,
    pub title: Option<String>,
    pub context: Option<Context>,
    pub metrics: Metrics,
    pub metadata: MetaData,
}

#[derive(Debug, Setters, Serialize, Deserialize, Clone)]
#[setters(into)]
pub struct MetaData {
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl MetaData {
    pub fn new(created_at: DateTime<Utc>) -> Self {
        Self { created_at, updated_at: None }
    }
}

impl Conversation {
    pub fn new(id: ConversationId) -> Self {
        let created_at = Utc::now();
        let metrics = Metrics::default().started_at(created_at);
        Self {
            id,
            metrics,
            metadata: MetaData::new(created_at),
            title: None,
            context: None,
        }
    }
    /// Creates a new conversation with a new conversation ID.
    ///
    /// This is a convenience constructor that automatically generates a unique
    /// conversation ID, making it easy to create new conversations without
    /// having to manually create the ID.
    pub fn generate() -> Self {
        Self::new(ConversationId::generate())
    }

    /// Generates an HTML representation of the conversation
    ///
    /// This method uses Handlebars to render the conversation as HTML
    /// from the template file, including all agents, events, and variables.
    ///
    /// # Errors
    /// - If the template file cannot be found or read
    /// - If the Handlebars template registration fails
    /// - If the template rendering fails
    pub fn to_html(&self) -> String {
        // Instead of using Handlebars, we now use our Element DSL
        crate::conversation_html::render_conversation_html(self)
    }

    /// Generates an HTML representation with related agent conversations
    ///
    /// Creates a single HTML document containing the main conversation
    /// and all related agent conversations with anchor links for navigation.
    pub fn to_html_with_related(&self, related: &[Conversation]) -> String {
        crate::conversation_html::render_conversation_html_with_related(self, related)
    }

    /// Returns a vector of user messages, selecting the first message from
    /// each consecutive sequence of user messages.
    pub fn first_user_messages(&self) -> Vec<&crate::ContextMessage> {
        self.context
            .as_ref()
            .map(|ctx| ctx.first_user_messages())
            .unwrap_or_default()
    }

    /// Returns the total token usage across all messages in the conversation.
    ///
    /// This is a convenience method that aggregates usage from the context,
    /// if available.
    pub fn accumulated_usage(&self) -> Option<crate::Usage> {
        self.context.as_ref().and_then(|ctx| ctx.accumulate_usage())
    }

    pub fn usage(&self) -> Option<crate::Usage> {
        self.context
            .iter()
            .flat_map(|ctx| ctx.messages.iter())
            .flat_map(|msg| msg.usage.into_iter())
            .last()
    }

    pub fn token_count(&self) -> Option<TokenCount> {
        self.context.as_ref().map(|ctx| ctx.token_count())
    }

    pub fn accumulated_cost(&self) -> Option<f64> {
        self.accumulated_usage().and_then(|usage| usage.cost)
    }

    /// Calculates the total cost including related conversations.
    ///
    /// This method sums the costs of the current conversation and all related
    /// conversations provided. It's useful for computing the total cost of a
    /// conversation tree when agent tools spawn child conversations.
    ///
    /// # Arguments
    ///
    /// * `conversations` - A slice of conversations to include in cost
    ///   calculation
    pub fn total_cost(conversations: &[Conversation]) -> Option<f64> {
        let costs: Vec<f64> = conversations
            .iter()
            .filter_map(|conv| conv.accumulated_cost())
            .collect();

        if costs.is_empty() {
            return None;
        }

        Some(costs.iter().sum())
    }

    /// Returns the number of messages in the conversation context.
    ///
    /// Returns `0` if the context has not been initialized yet.
    pub fn len(&self) -> usize {
        self.context
            .as_ref()
            .map(|ctx| ctx.messages.len())
            .unwrap_or(0)
    }

    /// Returns `true` if the conversation context has no messages.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Extracts all related conversation IDs from agent tool calls.
    ///
    /// This method scans through all tool results in the conversation's context
    /// and collects conversation IDs from AI tool values, which are created
    /// when agent tools are called and trigger new conversations.
    pub fn related_conversation_ids(&self) -> Vec<ConversationId> {
        self.context
            .as_ref()
            .map(|ctx| {
                ctx.messages
                    .iter()
                    .filter_map(|msg| msg.as_tool_result())
                    .flat_map(|result| &result.output.values)
                    .filter_map(|value| {
                        if let crate::ToolValue::AI { conversation_id, .. } = value {
                            Some(conversation_id)
                        } else {
                            None
                        }
                    })
                    .copied()
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{Context, ContextMessage, ToolOutput, ToolResult, ToolValue};

    #[test]
    fn test_related_conversation_ids_empty() {
        let conversation = Conversation::generate();
        let actual = conversation.related_conversation_ids();
        assert_eq!(actual, vec![]);
    }

    #[test]
    fn test_related_conversation_ids_with_agent_calls() {
        let agent_conv_id_1 = ConversationId::generate();
        let agent_conv_id_2 = ConversationId::generate();

        let context = Context::default()
            .add_message(ContextMessage::user("Test task", None))
            .add_message(ContextMessage::assistant("Working on it", None, None, None))
            .add_message(ContextMessage::Tool(ToolResult::new("agent_tool").output(
                Ok(ToolOutput {
                    is_error: false,
                    values: vec![ToolValue::AI {
                        value: "Agent result".to_string(),
                        conversation_id: agent_conv_id_1,
                    }],
                }),
            )))
            .add_message(ContextMessage::assistant("Continuing", None, None, None))
            .add_message(ContextMessage::Tool(
                ToolResult::new("another_agent").output(Ok(ToolOutput {
                    is_error: false,
                    values: vec![ToolValue::AI {
                        value: "Another agent result".to_string(),
                        conversation_id: agent_conv_id_2,
                    }],
                })),
            ));

        let conversation = Conversation::generate().context(context);
        let actual = conversation.related_conversation_ids();

        assert_eq!(actual.len(), 2);
        assert!(actual.contains(&agent_conv_id_1));
        assert!(actual.contains(&agent_conv_id_2));
    }

    #[test]
    fn test_related_conversation_ids_mixed_tool_results() {
        let agent_conv_id = ConversationId::generate();

        let context = Context::default()
            .add_message(ContextMessage::user("Test task", None))
            .add_message(ContextMessage::Tool(
                ToolResult::new("regular_tool").output(Ok(ToolOutput::text("Regular result"))),
            ))
            .add_message(ContextMessage::Tool(ToolResult::new("agent_tool").output(
                Ok(ToolOutput {
                    is_error: false,
                    values: vec![ToolValue::AI {
                        value: "Agent result".to_string(),
                        conversation_id: agent_conv_id,
                    }],
                }),
            )))
            .add_message(ContextMessage::Tool(
                ToolResult::new("another_regular_tool")
                    .output(Ok(ToolOutput::text("Another regular result"))),
            ));

        let conversation = Conversation::generate().context(context);
        let actual = conversation.related_conversation_ids();

        assert_eq!(actual, vec![agent_conv_id]);
    }

    #[test]
    fn test_total_cost() {
        use crate::{MessageEntry, Usage};

        // Create main conversation with cost
        // Create main conversation with cost
        let main_usage = Usage { cost: Some(0.01), ..Usage::default() };
        let main_entry: MessageEntry =
            ContextMessage::assistant("Response", None, None, None).into();
        let main_context = Context::default()
            .add_message(ContextMessage::user("Test", None))
            .add_entry(main_entry.usage(main_usage));

        let main_conv = Conversation::generate().context(main_context);

        // Create related conversations with costs
        let related_usage_1 = Usage { cost: Some(0.02), ..Usage::default() };
        let related_entry_1: MessageEntry =
            ContextMessage::assistant("Result 1", None, None, None).into();
        let related_context_1 = Context::default()
            .add_message(ContextMessage::user("Task 1", None))
            .add_entry(related_entry_1.usage(related_usage_1));

        let related_conv_1 = Conversation::generate().context(related_context_1);

        let related_usage_2 = Usage { cost: Some(0.03), ..Usage::default() };
        let related_entry_2: MessageEntry =
            ContextMessage::assistant("Result 2", None, None, None).into();
        let related_context_2 = Context::default()
            .add_message(ContextMessage::user("Task 2", None))
            .add_entry(related_entry_2.usage(related_usage_2));
        let related_conv_2 = Conversation::generate().context(related_context_2);

        let actual = Conversation::total_cost(&[main_conv, related_conv_1, related_conv_2]);

        // Check that cost is approximately 0.06 (accounting for floating point
        // precision)
        assert!(actual.is_some());
        assert!((actual.unwrap() - 0.06).abs() < 0.0001);
    }

    #[test]
    fn test_total_cost_no_costs() {
        let main_conv = Conversation::generate();
        let related_conv = Conversation::generate();

        let actual = Conversation::total_cost(&[main_conv, related_conv]);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_total_cost_partial_costs() {
        use crate::{MessageEntry, Usage};

        // Main conversation has no cost
        let main_conv = Conversation::generate();

        // Related conversation has cost
        let related_usage = Usage { cost: Some(0.05), ..Usage::default() };
        let related_entry: MessageEntry =
            ContextMessage::assistant("Result", None, None, None).into();
        let related_context = Context::default()
            .add_message(ContextMessage::user("Task", None))
            .add_entry(related_entry.usage(related_usage));

        let related_conv = Conversation::generate().context(related_context);

        let actual = Conversation::total_cost(&[main_conv, related_conv]);
        let expected = Some(0.05);

        assert_eq!(actual, expected);
    }
}
