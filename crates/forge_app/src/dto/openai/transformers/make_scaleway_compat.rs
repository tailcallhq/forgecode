use forge_domain::Transformer;

use crate::dto::openai::Request;

const GLM_5_2_MAX_OUTPUT_TOKENS: u32 = 16_384;

/// Adapts OpenAI-compatible requests to Scaleway Generative APIs limits.
pub(super) struct MakeScalewayCompat;

impl Transformer for MakeScalewayCompat {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        request.max_completion_tokens = request
            .max_completion_tokens
            .map(|tokens| tokens.min(GLM_5_2_MAX_OUTPUT_TOKENS));

        request.reasoning_effort = request
            .reasoning_effort
            .map(|effort| match effort.as_str() {
                "none" | "high" | "max" => effort,
                "xhigh" => "max".to_string(),
                _ => "high".to_string(),
            });

        request
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_clamps_max_completion_tokens_to_scaleway_limit() {
        let fixture = Request::default().max_completion_tokens(32_768);

        let actual = MakeScalewayCompat.transform(fixture);

        let expected = Some(GLM_5_2_MAX_OUTPUT_TOKENS);
        assert_eq!(actual.max_completion_tokens, expected);
    }

    #[test]
    fn test_preserves_max_completion_tokens_below_scaleway_limit() {
        let fixture = Request::default().max_completion_tokens(8_192);

        let actual = MakeScalewayCompat.transform(fixture);

        let expected = Some(8_192);
        assert_eq!(actual.max_completion_tokens, expected);
    }

    #[test]
    fn test_normalizes_reasoning_effort_to_scaleway_values() {
        let fixtures = [
            ("none", "none"),
            ("minimal", "high"),
            ("low", "high"),
            ("medium", "high"),
            ("high", "high"),
            ("xhigh", "max"),
            ("max", "max"),
        ];

        let actual = fixtures
            .into_iter()
            .map(|(input, _)| {
                MakeScalewayCompat
                    .transform(Request::default().reasoning_effort(input.to_string()))
                    .reasoning_effort
                    .unwrap()
            })
            .collect::<Vec<_>>();

        let expected = fixtures
            .into_iter()
            .map(|(_, output)| output.to_string())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
