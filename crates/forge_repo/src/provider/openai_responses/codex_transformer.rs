use std::borrow::Cow;

use async_openai::types::responses::{self as oai, CreateResponse};
use forge_domain::Transformer;

/// Transformer that adjusts Responses API requests for the Codex backend.
///
/// The Codex backend at `chatgpt.com/backend-api/codex/responses` differs from
/// the standard OpenAI Responses API in several ways:
/// - `store` **must** be `false` (the server defaults to `true` and rejects
///   omitted values).
/// - `temperature` is not supported and must be stripped.
/// - `max_output_tokens` is not supported and must be stripped.
/// - `include` always contains `reasoning.encrypted_content` for stateless
///   reasoning continuity.
/// - `text.verbosity` is forced to `Low` for concise output.
/// - User-provided `reasoning.effort` is preserved.
/// - `reasoning.summary` is forced to `Concise`.
pub struct CodexTransformer;

impl CodexTransformer {

    /// Rewrites response payloads that use Codex's `service_tier="fast"`
    /// alias to OpenAI's canonical `service_tier="priority"` value.
    ///
    /// This mirrors codex-rs behavior where Fast mode is serialized as
    /// `priority` for Responses API compatibility.
    pub(super) fn normalize_fast_service_tier_response_payload(payload: &str) -> Cow<'_, str> {
        if !payload.contains("\"service_tier\"") || !payload.contains("\"fast\"") {
            return Cow::Borrowed(payload);
        }

        let mut json = match serde_json::from_str::<serde_json::Value>(payload) {
            Ok(value) => value,
            Err(_) => return Cow::Borrowed(payload),
        };

        if !Self::rewrite_fast_service_tier(&mut json) {
            return Cow::Borrowed(payload);
        }

        serde_json::to_string(&json)
            .map(Cow::Owned)
            .unwrap_or_else(|_| Cow::Borrowed(payload))
    }

    fn rewrite_fast_service_tier(value: &mut serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                let mut changed = false;

                if let Some(service_tier) = map.get_mut("service_tier")
                    && let serde_json::Value::String(tier) = service_tier
                    && tier.eq_ignore_ascii_case("fast")
                {
                    *tier = "priority".to_string();
                    changed = true;
                }

                for nested in map.values_mut() {
                    changed |= Self::rewrite_fast_service_tier(nested);
                }

                changed
            }
            serde_json::Value::Array(items) => {
                items.iter_mut().any(Self::rewrite_fast_service_tier)
            }
            _ => false,
        }
    }
}

impl Transformer for CodexTransformer {
    type Value = CreateResponse;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        request.store = Some(false);
        request.temperature = None;
        request.max_output_tokens = None;
        request.top_p = None;

        let includes = request.include.get_or_insert_with(Vec::new);
        if !includes.contains(&oai::IncludeEnum::ReasoningEncryptedContent) {
            includes.push(oai::IncludeEnum::ReasoningEncryptedContent);
        }

        // Force text verbosity to Low for concise codex output
        let text = request.text.get_or_insert(oai::ResponseTextParam {
            format: oai::TextResponseFormatConfiguration::Text,
            verbosity: None,
        });
        text.verbosity = Some(oai::Verbosity::Low);

        if let Some(reasoning) = request.reasoning.as_mut() {
            reasoning.summary = Some(oai::ReasoningSummary::Concise);
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use async_openai::types::responses as oai;
    use forge_app::domain::ContextMessage;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::provider::FromDomain;

    fn fixture() -> CreateResponse {
        let context = forge_app::domain::Context::default()
            .add_message(ContextMessage::user("Hello", None))
            .max_tokens(1024usize)
            .temperature(forge_app::domain::Temperature::from(0.7));

        let mut req = oai::CreateResponse::from_domain(context).unwrap();
        req.model = Some("gpt-5.1-codex".to_string());
        req
    }

    #[test]
    fn test_codex_transformer_sets_store_false() {
        let fixture = fixture();
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.store, Some(false));
    }

    #[test]
    fn test_codex_transformer_strips_temperature() {
        let fixture = fixture();
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.temperature, None);
    }

    #[test]
    fn test_codex_transformer_strips_max_output_tokens() {
        let fixture = fixture();
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.max_output_tokens, None);
    }

    #[test]
    fn test_codex_transformer_includes_reasoning_encrypted_content() {
        let fixture = fixture();
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        let expected = vec![oai::IncludeEnum::ReasoningEncryptedContent];
        assert_eq!(actual.include, Some(expected));
    }

    #[test]
    fn test_codex_transformer_preserves_existing_includes_and_appends_reasoning_encrypted_content()
    {
        let mut fixture = fixture();
        fixture.include = Some(vec![oai::IncludeEnum::MessageOutputTextLogprobs]);
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        let expected = vec![
            oai::IncludeEnum::MessageOutputTextLogprobs,
            oai::IncludeEnum::ReasoningEncryptedContent,
        ];
        assert_eq!(actual.include, Some(expected));
    }

    #[test]
    fn test_codex_transformer_does_not_duplicate_reasoning_encrypted_content_include() {
        let mut fixture = fixture();
        fixture.include = Some(vec![oai::IncludeEnum::ReasoningEncryptedContent]);
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        let expected = vec![oai::IncludeEnum::ReasoningEncryptedContent];
        assert_eq!(actual.include, Some(expected));
    }

    #[test]
    fn test_codex_transformer_sets_text_verbosity_low() {
        let fixture = fixture();
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        let expected = Some(oai::Verbosity::Low);
        assert_eq!(
            actual.text.as_ref().and_then(|t| t.verbosity.clone()),
            expected
        );
    }

    #[test]
    fn test_codex_transformer_overrides_text_verbosity_to_low() {
        let mut fixture = fixture();
        fixture.text = Some(oai::ResponseTextParam {
            format: oai::TextResponseFormatConfiguration::Text,
            verbosity: Some(oai::Verbosity::High),
        });
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        let expected = Some(oai::Verbosity::Low);
        assert_eq!(
            actual.text.as_ref().and_then(|t| t.verbosity.clone()),
            expected
        );
    }

    #[test]
    fn test_codex_transformer_no_reasoning_unchanged() {
        let fixture = fixture();
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_codex_transformer_preserves_user_reasoning_effort() {
        let mut fixture = fixture();
        fixture.reasoning = Some(oai::Reasoning {
            effort: Some(oai::ReasoningEffort::Low),
            summary: Some(oai::ReasoningSummary::Auto),
        });
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        let actual = actual.reasoning.unwrap();
        assert_eq!(actual.effort, Some(oai::ReasoningEffort::Low));
        assert_eq!(actual.summary, Some(oai::ReasoningSummary::Concise));
    }

    #[test]
    fn test_codex_transformer_preserves_missing_reasoning_effort() {
        let mut fixture = fixture();
        fixture.reasoning = Some(oai::Reasoning {
            effort: None,
            summary: Some(oai::ReasoningSummary::Auto),
        });
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        let actual = actual.reasoning.unwrap();
        assert_eq!(actual.effort, None);
        assert_eq!(actual.summary, Some(oai::ReasoningSummary::Concise));
    }

    #[test]
    fn test_codex_transformer_preserves_other_fields() {
        let fixture = fixture();
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.model.as_deref(), Some("gpt-5.1-codex"));
        assert_eq!(actual.stream, Some(true));
    }

    #[test]
    fn test_normalize_fast_service_tier_response_payload_maps_fast_to_priority() {
        let fixture = r#"{"type":"response.completed","response":{"service_tier":"fast"}}"#;

        let actual = CodexTransformer::normalize_fast_service_tier_response_payload(fixture);

        let expected = serde_json::json!({
            "type": "response.completed",
            "response": {
                "service_tier": "priority"
            }
        });

        let actual_json: serde_json::Value = serde_json::from_str(actual.as_ref()).unwrap();
        assert_eq!(actual_json, expected);
    }

    #[test]
    fn test_normalize_fast_service_tier_response_payload_is_noop_without_fast() {
        let fixture = r#"{"type":"response.completed","response":{"service_tier":"flex"}}"#;

        let actual = CodexTransformer::normalize_fast_service_tier_response_payload(fixture);

        assert_eq!(actual, Cow::Borrowed(fixture));
    }
}
