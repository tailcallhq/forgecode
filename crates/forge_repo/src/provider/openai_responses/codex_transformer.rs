use async_openai::types::responses::{self as oai, CreateResponse, FunctionTool, Tool};
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

/// Transformer that sets defer_loading on MCP tools and injects tool_search.
///
/// This transformer is designed for GPT 5.4 models that support deferred tool
/// loading. It:
/// - Sets `defer_loading: Some(true)` on MCP tools (names starting with "mcp_")
/// - Injects a `tool_search` tool at the beginning of the tools list when there
///   are deferred tools (required by the API)
pub struct SetDeferLoading;

impl SetDeferLoading {
    /// Determines if a tool name represents an MCP tool that should be
    /// deferred.
    fn is_mcp_tool(name: &str) -> bool {
        name.starts_with("mcp_")
    }

    /// Creates the hosted tool_search tool required when deferred tools are
    /// present.
    ///
    /// Uses hosted (server-side) tool search so the API automatically searches
    /// the deferred tools declared in the request and returns the matching
    /// subset in the same response. No client-side search logic is needed.
    fn create_tool_search_tool() -> Tool {
        Tool::ToolSearch(oai::ToolSearchToolParam {
            execution: None,
            description: None,
            parameters: None,
        })
    }
}

impl Transformer for SetDeferLoading {
    type Value = CreateResponse;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        // Check if there are tools to process
        let Some(ref tools) = request.tools else {
            return request;
        };

        if tools.is_empty() {
            return request;
        }

        // Track if any tools will be deferred
        let mut has_deferred_tools = false;

        // Transform tools: set defer_loading for MCP tools
        let transformed_tools: Vec<Tool> = tools
            .iter()
            .filter(|tool| {
                // Filter out tool_search if it exists (we'll add our own)
                !matches!(tool, Tool::ToolSearch(_))
            })
            .map(|tool| {
                match tool {
                    Tool::Function(func_tool) => {
                        let should_defer = Self::is_mcp_tool(&func_tool.name);
                        if should_defer {
                            has_deferred_tools = true;
                        }
                        Tool::Function(FunctionTool {
                            name: func_tool.name.clone(),
                            parameters: func_tool.parameters.clone(),
                            strict: func_tool.strict,
                            description: func_tool.description.clone(),
                            defer_loading: Some(should_defer),
                        })
                    }
                    // Pass through other tool types unchanged
                    _ => tool.clone(),
                }
            })
            .collect();

        // If there are deferred tools, inject the tool_search tool at the beginning
        if has_deferred_tools {
            let mut new_tools = vec![Self::create_tool_search_tool()];
            new_tools.extend(transformed_tools);
            request.tools = Some(new_tools);
        } else {
            // No deferred tools, just use the transformed tools
            request.tools = Some(transformed_tools);
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

    // Tests for SetDeferLoading transformer

    fn create_test_function_tool(name: &str) -> Tool {
        Tool::Function(FunctionTool {
            name: name.to_string(),
            parameters: Some(serde_json::json!({"type": "object"})),
            strict: Some(true),
            description: Some(format!("Test tool: {}", name)),
            defer_loading: None,
        })
    }

    #[test]
    fn test_set_defer_loading_no_tools() {
        let mut request = CreateResponse::default();
        request.tools = None;

        let mut transformer = SetDeferLoading;
        let actual = transformer.transform(request);

        assert_eq!(actual.tools, None);
    }

    #[test]
    fn test_set_defer_loading_empty_tools() {
        let mut request = CreateResponse::default();
        request.tools = Some(vec![]);

        let mut transformer = SetDeferLoading;
        let actual = transformer.transform(request);

        assert_eq!(actual.tools, Some(vec![]));
    }

    #[test]
    fn test_set_defer_loading_only_builtin_tools() {
        let mut request = CreateResponse::default();
        request.tools = Some(vec![
            create_test_function_tool("read"),
            create_test_function_tool("write"),
            create_test_function_tool("shell"),
        ]);

        let mut transformer = SetDeferLoading;
        let actual = transformer.transform(request);

        let tools = actual.tools.unwrap();
        assert_eq!(tools.len(), 3);

        // All built-in tools should have defer_loading: Some(false)
        for tool in tools {
            if let Tool::Function(func) = tool {
                assert_eq!(func.defer_loading, Some(false));
            }
        }
    }

    #[test]
    fn test_set_defer_loading_mcp_tools_get_deferred() {
        let mut request = CreateResponse::default();
        request.tools = Some(vec![
            create_test_function_tool("read"),
            create_test_function_tool("mcp_github"),
            create_test_function_tool("write"),
            create_test_function_tool("mcp_gitlab"),
        ]);

        let mut transformer = SetDeferLoading;
        let actual = transformer.transform(request);

        let tools = actual.tools.unwrap();

        // Should have tool_search + 4 original tools = 5 total
        assert_eq!(tools.len(), 5);

        // First tool should be tool_search
        assert!(matches!(tools[0], Tool::ToolSearch(_)));

        // Check remaining tools
        for tool in &tools[1..] {
            if let Tool::Function(func) = tool {
                if func.name.starts_with("mcp_") {
                    assert_eq!(
                        func.defer_loading,
                        Some(true),
                        "MCP tool {} should be deferred",
                        func.name
                    );
                } else {
                    assert_eq!(
                        func.defer_loading,
                        Some(false),
                        "Built-in tool {} should not be deferred",
                        func.name
                    );
                }
            }
        }
    }

    #[test]
    fn test_set_defer_loading_filters_existing_tool_search() {
        let mut request = CreateResponse::default();
        let tool_search = SetDeferLoading::create_tool_search_tool();
        request.tools = Some(vec![tool_search, create_test_function_tool("mcp_github")]);

        let mut transformer = SetDeferLoading;
        let actual = transformer.transform(request);

        let tools = actual.tools.unwrap();

        // Should still have only 2 tools (tool_search + mcp_github)
        // because existing tool_search is filtered and replaced
        assert_eq!(tools.len(), 2);

        // First tool should be tool_search (the new one we created)
        assert!(matches!(tools[0], Tool::ToolSearch(_)));
    }

    #[test]
    fn test_set_defer_loading_preserves_non_function_tools() {
        // Create a request with only built-in tools (no MCP tools)
        let mut request = CreateResponse::default();
        request.tools = Some(vec![
            create_test_function_tool("read"),
            create_test_function_tool("write"),
        ]);

        let mut transformer = SetDeferLoading;
        let actual = transformer.transform(request);

        // Should have 2 tools (no tool_search since no MCP tools)
        let tools = actual.tools.unwrap();
        assert_eq!(tools.len(), 2);

        // Both should be Function tools with defer_loading: Some(false)
        for tool in tools {
            if let Tool::Function(func) = tool {
                assert_eq!(func.defer_loading, Some(false));
            } else {
                panic!("Expected Function tool, got something else");
            }
        }
    }
}
