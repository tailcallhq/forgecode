use forge_domain::Transformer;

use crate::dto::openai::Request;

/// makes the Request compatible with the OpenAI API.
pub struct MakeOpenAiCompat;

impl Transformer for MakeOpenAiCompat {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        // remove fields that are not supported by open-ai.
        request.provider = None;
        request.transforms = None;
        request.prompt = None;
        request.models = None;
        request.route = None;
        request.top_k = None;
        request.top_p = None;
        request.repetition_penalty = None;
        request.min_p = None;
        request.top_a = None;
        request.session_id = None;
        request.reasoning = None;
        request.reasoning_or = None;

        let tools_present = request
            .tools
            .as_ref()
            .is_some_and(|tools| !tools.is_empty());

        if !tools_present {
            // drop `parallel_tool_calls` field if tools are not passed to the request.
            request.parallel_tool_calls = None;
        }

        // OpenAI has deprecated `max_tokens`, now it is `max_completion_tokens`.
        request.max_completion_tokens = request.max_tokens.take();

        request
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_parallel_tool_calls_removed_when_no_tools() {
        use crate::dto::openai::Request;

        let fixture = Request::default().parallel_tool_calls(true);
        let mut transformer = MakeOpenAiCompat;
        let actual = transformer.transform(fixture);
        let expected = None;
        assert_eq!(actual.parallel_tool_calls, expected);
    }

    #[test]
    fn test_parallel_tool_calls_removed_when_empty_tools() {
        let fixture = Request::default().tools(vec![]).parallel_tool_calls(true);
        let mut transformer = MakeOpenAiCompat;
        let actual = transformer.transform(fixture);
        let expected = None;
        assert_eq!(actual.parallel_tool_calls, expected);
    }

    #[test]
    fn test_parallel_tool_calls_preserved_when_tools_present() {
        use crate::dto::openai::{FunctionDescription, FunctionType, Request, Tool};

        let fixture = Request::default()
            .tools(vec![Tool {
                r#type: FunctionType,
                function: FunctionDescription {
                    description: Some("test".to_string()),
                    name: "test".to_string(),
                    parameters: serde_json::json!({}),
                },
            }])
            .parallel_tool_calls(true);
        let mut transformer = MakeOpenAiCompat;
        let actual = transformer.transform(fixture);
        let expected = Some(true);
        assert_eq!(actual.parallel_tool_calls, expected);
    }

    #[test]
    fn test_reasoning_removed() {
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: Some(true),
            effort: None,
            max_tokens: None,
            exclude: None,
        });
        let mut transformer = MakeOpenAiCompat;
        let actual = transformer.transform(fixture);
        let expected = None;
        assert_eq!(actual.reasoning, expected);
    }

    #[test]
    fn test_max_tokens_mapped_correctly() {
        let fixture = Request::default().max_tokens(100);
        let mut transformer = MakeOpenAiCompat;
        let actual = transformer.transform(fixture);
        let expected = Some(100);
        assert_eq!(actual.max_completion_tokens, expected);
        assert_eq!(actual.max_tokens, None);
    }
}
