use derive_setters::Setters;
use merge::Merge;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{Context, ModelId, Role};

/// Configuration for automatic context compaction
#[derive(Debug, Clone, Serialize, Deserialize, Merge, Setters, JsonSchema, PartialEq)]
#[setters(strip_option, into)]
pub struct Compact {
    /// Number of most recent messages to preserve during compaction.
    /// These messages won't be considered for summarization. Works alongside
    /// eviction_window - the more conservative limit (fewer messages to
    /// compact) takes precedence.
    #[merge(strategy = crate::merge::std::overwrite)]
    #[serde(default)]
    pub retention_window: usize,

    /// Maximum percentage of the context that can be summarized during
    /// compaction. Valid values are between 0.0 and 1.0, where 0.0 means no
    /// compaction and 1.0 allows summarizing all messages. Works alongside
    /// retention_window - the more conservative limit (fewer messages to
    /// compact) takes precedence.
    #[merge(strategy = crate::merge::std::overwrite)]
    #[serde(default, deserialize_with = "deserialize_percentage")]
    pub eviction_window: f64,

    /// Maximum number of tokens to keep after compaction
    #[merge(strategy = crate::merge::option)]
    pub max_tokens: Option<usize>,

    /// Maximum number of tokens before triggering compaction. This acts as an
    /// absolute cap and is combined with
    /// `token_threshold_percentage` by taking the lower value.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[merge(strategy = crate::merge::option)]
    pub token_threshold: Option<usize>,

    /// Maximum percentage of the model context window used to derive the token
    /// threshold before triggering compaction. This is combined with
    /// `token_threshold` by taking the lower value.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_percentage"
    )]
    #[merge(strategy = crate::merge::option)]
    pub token_threshold_percentage: Option<f64>,

    /// Maximum number of conversation turns before triggering compaction
    #[serde(skip_serializing_if = "Option::is_none")]
    #[merge(strategy = crate::merge::option)]
    pub turn_threshold: Option<usize>,

    /// Maximum number of messages before triggering compaction
    #[serde(skip_serializing_if = "Option::is_none")]
    #[merge(strategy = crate::merge::option)]
    pub message_threshold: Option<usize>,

    /// Model ID to use for compaction, useful when compacting with a
    /// cheaper/faster model. If not specified, the root level model will be
    /// used.
    #[merge(strategy = crate::merge::option)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelId>,
    /// Whether to trigger compaction when the last message is from a user
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[merge(strategy = crate::merge::option)]
    pub on_turn_end: Option<bool>,
}

fn deserialize_percentage<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let value = f64::deserialize(deserializer)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(Error::custom(format!(
            "percentage must be between 0.0 and 1.0, got {value}"
        )));
    }
    Ok(value)
}

fn deserialize_optional_percentage<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let value = Option::<f64>::deserialize(deserializer)?;
    if let Some(value) = value
        && !(0.0..=1.0).contains(&value)
    {
        return Err(Error::custom(format!(
            "percentage must be between 0.0 and 1.0, got {value}"
        )));
    }
    Ok(value)
}

impl Default for Compact {
    fn default() -> Self {
        Self::new()
    }
}

impl Compact {
    /// Creates a new compaction configuration with the specified maximum token
    /// limit
    pub fn new() -> Self {
        Self {
            max_tokens: None,
            token_threshold: None,
            token_threshold_percentage: None,
            turn_threshold: None,
            message_threshold: None,
            model: None,
            eviction_window: 0.2, // Default to 20% compaction
            retention_window: 0,
            on_turn_end: None,
        }
    }

    /// Determines if compaction should be triggered based on the current
    /// context
    pub fn should_compact(&self, context: &Context, token_count: usize) -> bool {
        self.should_compact_due_to_tokens(token_count)
            || self.should_compact_due_to_turns(context)
            || self.should_compact_due_to_messages(context)
            || self.should_compact_on_turn_end(context)
    }

    /// Checks if compaction should be triggered due to token count exceeding
    /// threshold
    fn should_compact_due_to_tokens(&self, token_count: usize) -> bool {
        if let Some(token_threshold) = self.token_threshold {
            debug!(tokens = ?token_count, "Token count");
            // use provided prompt_tokens if available, otherwise estimate token count
            token_count >= token_threshold
        } else {
            false
        }
    }

    /// Checks if compaction should be triggered due to turn count exceeding
    /// threshold
    fn should_compact_due_to_turns(&self, context: &Context) -> bool {
        if let Some(turn_threshold) = self.turn_threshold {
            context
                .messages
                .iter()
                .filter(|message| message.has_role(Role::User))
                .count()
                >= turn_threshold
        } else {
            false
        }
    }

    /// Checks if compaction should be triggered due to message count exceeding
    /// threshold
    fn should_compact_due_to_messages(&self, context: &Context) -> bool {
        if let Some(message_threshold) = self.message_threshold {
            // Count messages directly from context
            let msg_count = context.messages.len();
            msg_count >= message_threshold
        } else {
            false
        }
    }

    /// Checks if compaction should be triggered when the last message is from a
    /// user
    fn should_compact_on_turn_end(&self, context: &Context) -> bool {
        if let Some(true) = self.on_turn_end {
            context
                .messages
                .last()
                .map(|message| message.has_role(Role::User))
                .unwrap_or(false)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::MessagePattern;

    /// Creates a Context from a condensed string pattern where:
    /// - 'u' = User message
    /// - 'a' = Assistant message
    /// - 's' = System message Example: ctx("uau") creates User -> Assistant ->
    ///   User messages
    fn ctx(pattern: &str) -> Context {
        MessagePattern::new(pattern).build()
    }

    #[test]
    fn test_should_compact_due_to_tokens_exceeds_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(100_usize);
        let actual = fixture.should_compact_due_to_tokens(150);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_tokens_under_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(100_usize);
        let actual = fixture.should_compact_due_to_tokens(50);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_tokens_equals_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(100_usize);
        let actual = fixture.should_compact_due_to_tokens(100);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_tokens_no_threshold() {
        let fixture = Compact::new().model(ModelId::new("test-model"));
        let actual = fixture.should_compact_due_to_tokens(1000);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_turns_exceeds_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .turn_threshold(2_usize);
        let context = ctx("uauau");

        let actual = fixture.should_compact_due_to_turns(&context);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_turns_under_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .turn_threshold(3_usize);
        let context = ctx("ua");
        let actual = fixture.should_compact_due_to_turns(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_turns_equals_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .turn_threshold(2_usize);
        let context = ctx("uau");
        let actual = fixture.should_compact_due_to_turns(&context);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_turns_no_threshold() {
        let fixture = Compact::new().model(ModelId::new("test-model"));
        let context = ctx("uuu");
        let actual = fixture.should_compact_due_to_turns(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_turns_ignores_non_user_messages() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .turn_threshold(2_usize);
        let context = ctx("uasa");
        let actual = fixture.should_compact_due_to_turns(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_messages_exceeds_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .message_threshold(3_usize);
        let context = ctx("uaua");
        let actual = fixture.should_compact_due_to_messages(&context);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_messages_under_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .message_threshold(5_usize);
        let context = ctx("ua");
        let actual = fixture.should_compact_due_to_messages(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_messages_equals_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .message_threshold(3_usize);
        let context = ctx("uau");
        let actual = fixture.should_compact_due_to_messages(&context);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_messages_no_threshold() {
        let fixture = Compact::new().model(ModelId::new("test-model"));
        let context = ctx("uauau");
        let actual = fixture.should_compact_due_to_messages(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_no_thresholds_set() {
        let fixture = Compact::new().model(ModelId::new("test-model"));
        let context = ctx("ua");
        let actual = fixture.should_compact(&context, 1000);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_token_threshold_triggers() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(100_usize);
        let context = ctx("u");
        let actual = fixture.should_compact(&context, 150);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_turn_threshold_triggers() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .turn_threshold(1_usize);
        let context = ctx("uau");
        let actual = fixture.should_compact(&context, 50);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_message_threshold_triggers() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .message_threshold(2_usize);
        let context = ctx("uau");
        let actual = fixture.should_compact(&context, 50);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_multiple_thresholds_any_triggers() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(200_usize)
            .turn_threshold(5_usize)
            .message_threshold(10_usize);
        let context = ctx("ua");
        let actual = fixture.should_compact(&context, 250); // Only token threshold exceeded
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_multiple_thresholds_none_trigger() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(200_usize)
            .turn_threshold(5_usize)
            .message_threshold(10_usize);
        let context = ctx("ua");
        let actual = fixture.should_compact(&context, 100); // All thresholds under limit
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_empty_context() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .message_threshold(1_usize);
        let context = ctx("");
        let actual = fixture.should_compact(&context, 0);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_enabled_user_last() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(true);
        let context = ctx("au");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_enabled_assistant_last() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(true);
        let context = ctx("ua");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_enabled_system_last() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(true);
        let context = ctx("us");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_disabled() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(false);
        let context = ctx("au");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_not_configured() {
        let fixture = Compact::new().model(ModelId::new("test-model")); // No configuration set
        let context = ctx("au");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_empty_context() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(true);
        let context = ctx("");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_last_user_message_integration() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(true);
        let context = ctx("au");
        let actual = fixture.should_compact(&context, 10); // Low token count, no other thresholds
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_last_user_message_integration_disabled() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(false);
        let context = ctx("au");
        let actual = fixture.should_compact(&context, 10); // Low token count, no other thresholds
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_multiple_conditions_with_last_user_message() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(200_usize)
            .on_turn_end(true);
        let context = ctx("au");
        let actual = fixture.should_compact(&context, 50); // Token threshold not met, but last message is user
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compact_model_none_falls_back_to_agent_model() {
        // Fixture
        let compact = Compact::new()
            .token_threshold(1000_usize)
            .turn_threshold(5_usize);

        // Assert
        assert_eq!(compact.model, None);
        assert_eq!(compact.token_threshold, Some(1000_usize));
        assert_eq!(compact.turn_threshold, Some(5_usize));
    }
}
