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
/// - `reasoning.effort` and `reasoning.summary` are passed through as-is from
///   the caller.
pub struct CodexTransformer;

impl Transformer for CodexTransformer {
    type Value = CreateResponse;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        request.store = Some(false);
        request.temperature = None;
        request.max_output_tokens = None;

        let includes = request.include.get_or_insert_with(Vec::new);
        if !includes.contains(&oai::IncludeEnum::ReasoningEncryptedContent) {
            includes.push(oai::IncludeEnum::ReasoningEncryptedContent);
        }

        request
    }
}

#[cfg(test)]
mod tests {
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
    fn test_codex_transformer_preserves_reasoning_effort_and_summary() {
        let reasoning = oai::Reasoning {
            effort: Some(oai::ReasoningEffort::Low),
            summary: Some(oai::ReasoningSummary::Detailed),
        };

        let mut fixture = fixture();
        fixture.reasoning = Some(reasoning);
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        assert_eq!(
            actual.reasoning.as_ref().and_then(|r| r.effort.clone()),
            Some(oai::ReasoningEffort::Low)
        );
        assert_eq!(
            actual.reasoning.as_ref().and_then(|r| r.summary),
            Some(oai::ReasoningSummary::Detailed)
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
    fn test_codex_transformer_preserves_other_fields() {
        let fixture = fixture();
        let mut transformer = CodexTransformer;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.model.as_deref(), Some("gpt-5.1-codex"));
        assert_eq!(actual.stream, Some(true));
    }
}
