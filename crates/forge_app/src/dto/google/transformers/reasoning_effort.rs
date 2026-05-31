use forge_domain::Transformer;

use crate::dto::google::request::{Part, Role};
use crate::dto::google::{Level, Request};

/// The substring injected by the orchestrator when it demands verification.
const VERIFICATION_REMINDER_MARKER: &str = "verification-specialist";

pub struct ReasoningEffort;

impl ReasoningEffort {
    /// Returns `true` if any user-role content part in the request contains
    /// the verification-specialist reminder injected by the orchestrator.
    fn verification_reminder_sent(request: &Request) -> bool {
        request
            .contents
            .iter()
            .filter(|c| c.role == Some(Role::User))
            .flat_map(|c| c.parts.iter())
            .any(|part| {
                if let Part::Text { text, .. } = part {
                    text.contains(VERIFICATION_REMINDER_MARKER)
                } else {
                    false
                }
            })
    }

    /// Determines the reasoning effort level based on conversation progress.
    ///
    /// - First 5 assistant messages: `High` (warm-up, needs full reasoning).
    /// - After that, until the verification reminder is sent: `Low` (routine
    ///   work).
    /// - Once the verification reminder has been injected: `High` (final
    ///   review).
    fn determine_level(request: &Request) -> Level {
        let assistant_msg_count = request
            .contents
            .iter()
            .filter(|c| c.role == Some(Role::Model))
            .count();

        #[allow(clippy::if_same_then_else)]
        if assistant_msg_count < 5 {
            Level::High
        } else if Self::verification_reminder_sent(request) {
            Level::High
        } else {
            Level::Low
        }
    }
}

impl Transformer for ReasoningEffort {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        let level = Self::determine_level(&request);

        if let Some(generation_config) = &mut request.generation_config
            && let Some(thinking_config) = &mut generation_config.thinking_config
        {
            thinking_config.thinking_level = Some(level);
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, ContextMessage, ReasoningConfig};
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::dto::google::Request;

    fn fixture_context(assistant_count: usize, with_verification_reminder: bool) -> Context {
        let mut context = Context {
            reasoning: Some(ReasoningConfig {
                enabled: Some(true),
                max_tokens: Some(1024),
                ..Default::default()
            }),
            ..Default::default()
        };

        for i in 0..assistant_count {
            context = context
                .add_message(ContextMessage::user(format!("Q{i}"), None))
                .add_message(ContextMessage::assistant(format!("A{i}"), None, None, None));
        }

        if with_verification_reminder {
            context = context.add_message(ContextMessage::user(
                "You have NOT yet invoked the `verification-specialist` skill.",
                None,
            ));
        }

        context
    }

    fn thinking_level(context: Context) -> Option<Level> {
        let request = Request::from(context);
        let mut transformer = ReasoningEffort;
        transformer
            .transform(request)
            .generation_config?
            .thinking_config?
            .thinking_level
    }

    #[test]
    fn test_high_for_first_5_messages() {
        // 4 assistant messages — still in the initial High window
        let actual = thinking_level(fixture_context(4, false));
        assert_eq!(actual, Some(Level::High));
    }

    #[test]
    fn test_high_at_exactly_4_messages() {
        // boundary: 4 < 5, so High
        let actual = thinking_level(fixture_context(4, false));
        assert_eq!(actual, Some(Level::High));
    }

    #[test]
    fn test_low_after_5_messages_without_reminder() {
        // 5 assistant messages, no reminder yet — Low
        let actual = thinking_level(fixture_context(5, false));
        assert_eq!(actual, Some(Level::Low));
    }

    #[test]
    fn test_low_for_many_messages_without_reminder() {
        // 30 messages, still no reminder — Low
        let actual = thinking_level(fixture_context(30, false));
        assert_eq!(actual, Some(Level::Low));
    }

    #[test]
    fn test_high_after_verification_reminder_sent() {
        // 20 assistant messages but reminder was injected — High
        let actual = thinking_level(fixture_context(20, true));
        assert_eq!(actual, Some(Level::High));
    }

    #[test]
    fn test_high_when_reminder_sent_even_early() {
        // Reminder before the 5-message threshold — still High (already High anyway)
        let actual = thinking_level(fixture_context(2, true));
        assert_eq!(actual, Some(Level::High));
    }

    #[test]
    fn test_noop_without_thinking_config() {
        // No reasoning config — thinking_level stays None
        let context = Context::default().add_message(ContextMessage::user("Q", None));
        let actual = thinking_level(context);
        assert_eq!(actual, None);
    }
}
