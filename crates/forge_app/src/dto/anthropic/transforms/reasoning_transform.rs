use forge_domain::{Context, Transformer};

pub struct ReasoningTransform;

impl Transformer for ReasoningTransform {
    type Value = Context;
    fn transform(&mut self, mut context: Self::Value) -> Self::Value {
        // Must stay in lockstep with the Anthropic request builder, which gates
        // on the same predicate — otherwise `thinking`/`output_config` ship
        // alongside sampling params that Anthropic rejects.
        if context.is_reasoning_supported() {
            context.top_k = None;
            context.top_p = None;
        }

        context
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, ReasoningConfig, TopK, TopP, Transformer};
    use pretty_assertions::assert_eq;

    use super::*;

    fn create_context_fixture() -> Context {
        Context::default()
            .top_k(TopK::new(50).unwrap())
            .top_p(TopP::new(0.8).unwrap())
    }

    fn create_reasoning_config_fixture(
        enabled: bool,
        max_tokens: Option<usize>,
    ) -> ReasoningConfig {
        ReasoningConfig {
            enabled: Some(enabled),
            max_tokens,
            effort: None,
            exclude: None,
        }
    }

    #[test]
    fn test_reasoning_enabled_with_max_tokens_removes_top_k_and_top_p() {
        let fixture =
            create_context_fixture().reasoning(create_reasoning_config_fixture(true, Some(1024)));
        let mut transformer = ReasoningTransform;
        let actual = transformer.transform(fixture);
        let expected =
            Context::default().reasoning(create_reasoning_config_fixture(true, Some(1024)));

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_reasoning_disabled_preserves_top_k_and_top_p() {
        let fixture =
            create_context_fixture().reasoning(create_reasoning_config_fixture(false, Some(1024)));
        let mut transformer = ReasoningTransform;
        let actual = transformer.transform(fixture.clone());
        let expected = fixture;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_reasoning_enabled_without_max_tokens_removes_top_k_and_top_p() {
        let fixture =
            create_context_fixture().reasoning(create_reasoning_config_fixture(true, None));
        let mut transformer = ReasoningTransform;
        let actual = transformer.transform(fixture);
        let expected = Context::default().reasoning(create_reasoning_config_fixture(true, None));

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_no_reasoning_config_preserves_top_k_and_top_p() {
        let fixture = create_context_fixture();
        let mut transformer = ReasoningTransform;
        let actual = transformer.transform(fixture.clone());
        let expected = fixture;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_enabled_none_with_effort_still_strips_top_k_and_top_p() {
        // `enabled: None` + effort is treated as reasoning-on (domain rule).
        let fixture = create_context_fixture().reasoning(ReasoningConfig {
            enabled: None,
            max_tokens: None,
            effort: Some(forge_domain::Effort::High),
            exclude: None,
        });
        let mut transformer = ReasoningTransform;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.top_k, None);
        assert_eq!(actual.top_p, None);
    }

    #[test]
    fn test_enabled_none_with_positive_max_tokens_still_strips_top_k_and_top_p() {
        let fixture = create_context_fixture().reasoning(ReasoningConfig {
            enabled: None,
            max_tokens: Some(8000),
            effort: None,
            exclude: None,
        });
        let mut transformer = ReasoningTransform;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.top_k, None);
        assert_eq!(actual.top_p, None);
    }

    #[test]
    fn test_enabled_none_with_zero_max_tokens_preserves_top_k_and_top_p() {
        // Matches `is_reasoning_supported`: max_tokens == 0 is treated as off.
        let fixture = create_context_fixture().reasoning(ReasoningConfig {
            enabled: None,
            max_tokens: Some(0),
            effort: None,
            exclude: None,
        });
        let mut transformer = ReasoningTransform;
        let actual = transformer.transform(fixture.clone());

        assert_eq!(actual.top_k, fixture.top_k);
        assert_eq!(actual.top_p, fixture.top_p);
    }
}
