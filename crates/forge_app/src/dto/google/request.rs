use derive_setters::Setters;
use forge_domain::{Context, ContextMessage};
use serde::Serialize;

#[derive(Serialize, Default, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<Content>,
    pub contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<ToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<SafetySetting>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<serde_json::Value>,
}

#[derive(Serialize, Default, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_config: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_modalities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_timestamp: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_resolution: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_config: Option<serde_json::Value>,
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<Level>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_thoughts: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Minimal,
    Low,
    Medium,
    High,
}

#[derive(Serialize)]
pub struct Content {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    pub parts: Vec<Part>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum Part {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thought: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    Image {
        inline_data: ImageSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    FunctionCall {
        function_call: FunctionCallData,
        #[serde(skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
    FunctionResponse {
        function_response: FunctionResponseData,
    },
    FileData {
        file_data: FileDataInfo,
    },
}

#[derive(Serialize)]
pub struct ImageSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCallData {
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionResponseData {
    pub name: String,
    pub response: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDataInfo {
    pub mime_type: String,
    pub file_uri: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CacheControl {
    Ephemeral,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Model,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum Tool {
    FunctionDeclarations {
        #[serde(skip_serializing_if = "Option::is_none")]
        function_declarations: Option<Vec<FunctionDeclaration>>,
    },
    GoogleSearch {
        google_search: GoogleSearchTool,
    },
    GoogleSearchRetrieval {
        google_search_retrieval: GoogleSearchRetrievalTool,
    },
    EnterpriseWebSearch {
        enterprise_web_search: serde_json::Value,
    },
    UrlContext {
        url_context: serde_json::Value,
    },
    CodeExecution {
        code_execution: serde_json::Value,
    },
    FileSearch {
        file_search: FileSearchTool,
    },
    GoogleMaps {
        google_maps: serde_json::Value,
    },
    Retrieval {
        retrieval: RetrievalTool,
    },
}

#[derive(Serialize, Default, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct GoogleSearchTool {
    // Empty object for Gemini 2.0+
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct GoogleSearchRetrievalTool {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_retrieval_config: Option<DynamicRetrievalConfig>,
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct DynamicRetrievalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_threshold: Option<f64>,
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct FileSearchTool {
    pub file_search_store_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_filter: Option<String>,
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalTool {
    pub vertex_rag_store: VertexRagStore,
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct VertexRagStore {
    pub rag_resources: RagResources,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_top_k: Option<i32>,
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct RagResources {
    pub rag_corpus: String,
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDeclaration {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_calling_config: Option<FunctionCallingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_config: Option<serde_json::Value>,
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCallingConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<FunctionCallingMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_function_names: Option<Vec<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FunctionCallingMode {
    ModeUnspecified,
    Auto,
    Any,
    None,
}

#[derive(Serialize, Setters)]
#[setters(into, strip_option)]
#[serde(rename_all = "camelCase")]
pub struct SafetySetting {
    pub category: String,
    pub threshold: String,
}

impl From<Context> for Request {
    fn from(context: Context) -> Self {
        // Extract system instruction from ALL system messages
        let system_parts: Vec<Part> = context
            .messages
            .iter()
            .filter(|msg| msg.has_role(forge_domain::Role::System))
            .filter_map(|msg| {
                msg.content().map(|content| Part::Text {
                    text: content.to_string(),
                    thought: None,
                    thought_signature: None,
                    cache_control: None,
                })
            })
            .collect();

        let system_instruction = if !system_parts.is_empty() {
            Some(Content { role: None, parts: system_parts })
        } else {
            None
        };

        // Convert messages (excluding system messages)
        // Group consecutive tool results into single Content objects to match Google's
        // API requirements
        let mut contents: Vec<Content> = Vec::new();
        let mut pending_tool_parts: Vec<Part> = Vec::new();

        for msg in context
            .messages
            .into_iter()
            .filter(|msg| !msg.has_role(forge_domain::Role::System))
        {
            match msg.message {
                ContextMessage::Tool(tool_result) => {
                    // Collect tool result parts to be grouped together
                    pending_tool_parts.push(Part::from(tool_result));
                }
                other => {
                    // Flush any pending tool results first
                    if !pending_tool_parts.is_empty() {
                        contents.push(Content {
                            role: Some(Role::User),
                            parts: std::mem::take(&mut pending_tool_parts),
                        });
                    }

                    // Add the current non-tool message
                    let content = Content::from(other);
                    if !content.parts.is_empty() {
                        contents.push(content);
                    }
                }
            }
        }

        // Flush any remaining tool results
        if !pending_tool_parts.is_empty() {
            contents.push(Content { role: Some(Role::User), parts: pending_tool_parts });
        }

        // Convert tools
        let tools = if !context.tools.is_empty() {
            Some(vec![Tool::FunctionDeclarations {
                function_declarations: Some(
                    context
                        .tools
                        .into_iter()
                        .map(FunctionDeclaration::from)
                        .collect(),
                ),
            }])
        } else {
            None
        };

        // Build generation config
        let generation_config = Some(GenerationConfig {
            max_output_tokens: context.max_tokens.map(|t| t as i32),
            temperature: context.temperature.map(|t| t.value() as f64),
            top_p: context.top_p.map(|t| t.value() as f64),
            top_k: context.top_k.map(|t| t.value() as i32),
            response_mime_type: context.response_format.as_ref().and_then(|rf| match rf {
                forge_domain::ResponseFormat::JsonSchema(_) => Some("application/json".to_string()),
                _ => None,
            }),
            response_schema: context.response_format.and_then(|rf| match rf {
                forge_domain::ResponseFormat::JsonSchema(schema) => {
                    let mut schema_value = serde_json::to_value(*schema).ok()?;
                    // Sanitize schema for Gemini API compatibility
                    crate::utils::sanitize_gemini_schema(&mut schema_value);
                    Some(schema_value)
                }
                _ => None,
            }),
            thinking_config: context.reasoning.and_then(|reasoning| {
                reasoning.enabled.and_then(|enabled| {
                    if enabled {
                        Some(ThinkingConfig {
                            thinking_level: None,
                            thinking_budget: reasoning.max_tokens.map(|t| t as i32),
                            include_thoughts: Some(true),
                        })
                    } else {
                        None
                    }
                })
            }),
            ..Default::default()
        });

        // Build tool config for tool choice
        // Only set tool_config if there's an explicit tool choice
        let tool_config = context.tool_choice.map(ToolConfig::from);

        Request {
            system_instruction,
            contents,
            generation_config,
            tools,
            tool_config,
            safety_settings: None,
            cached_content: None,
            labels: None,
        }
    }
}

impl From<forge_domain::ToolChoice> for ToolConfig {
    fn from(choice: forge_domain::ToolChoice) -> Self {
        let (mode, allowed_function_names) = match choice {
            forge_domain::ToolChoice::Auto => (FunctionCallingMode::Auto, None),
            forge_domain::ToolChoice::None => (FunctionCallingMode::None, None),
            forge_domain::ToolChoice::Required => (FunctionCallingMode::Any, None),
            forge_domain::ToolChoice::Call(name) => {
                (FunctionCallingMode::Any, Some(vec![name.to_string()]))
            }
        };

        ToolConfig {
            function_calling_config: Some(FunctionCallingConfig {
                mode: Some(mode),
                allowed_function_names,
            }),
            retrieval_config: None,
        }
    }
}

impl From<forge_domain::ToolDefinition> for FunctionDeclaration {
    fn from(tool: forge_domain::ToolDefinition) -> Self {
        let mut parameters =
            serde_json::to_value(tool.input_schema).unwrap_or(serde_json::json!({}));

        // Sanitize schema for Gemini API compatibility (strips $schema,
        // removes additionalProperties, converts integer enums, ensures
        // arrays have items, removes properties from non-objects, etc.)
        crate::utils::sanitize_gemini_schema(&mut parameters);

        FunctionDeclaration {
            name: tool.name.to_string(),
            description: Some(tool.description),
            parameters,
        }
    }
}

impl From<ContextMessage> for Content {
    fn from(message: ContextMessage) -> Self {
        match message {
            ContextMessage::Text(text_message) => Content::from(text_message),
            ContextMessage::Tool(tool_result) => Content::from(tool_result),
            ContextMessage::Image(image) => Content::from(image),
        }
    }
}

impl From<forge_domain::TextMessage> for Content {
    fn from(text_message: forge_domain::TextMessage) -> Self {
        let role = match text_message.role {
            forge_domain::Role::User => Some(Role::User),
            forge_domain::Role::Assistant => Some(Role::Model),
            forge_domain::Role::System => None, // System messages are handled separately
        };

        let mut parts = Vec::new();

        // Add text part if content is not empty
        if !text_message.content.is_empty() {
            parts.push(Part::Text {
                text: text_message.content,
                thought: None,
                thought_signature: text_message.thought_signature.clone(),
                cache_control: None,
            });
        }

        // Add function calls if present
        if let Some(tool_calls) = text_message.tool_calls {
            parts.extend(tool_calls.into_iter().map(Part::from));
        }

        Content { role, parts }
    }
}

impl From<forge_domain::ToolResult> for Content {
    fn from(tool_result: forge_domain::ToolResult) -> Self {
        Content {
            role: Some(Role::User), // Tool results come back as user messages in Google's API
            parts: vec![Part::from(tool_result)],
        }
    }
}

impl From<forge_domain::Image> for Content {
    fn from(image: forge_domain::Image) -> Self {
        Content { role: Some(Role::User), parts: vec![Part::from(image)] }
    }
}

impl From<forge_domain::ToolCallFull> for Part {
    fn from(tool_call: forge_domain::ToolCallFull) -> Self {
        Part::FunctionCall {
            function_call: FunctionCallData {
                name: tool_call.name.to_string(),
                args: tool_call.arguments.parse().unwrap_or(serde_json::json!({})),
            },
            thought_signature: tool_call.thought_signature,
        }
    }
}

impl From<forge_domain::ToolResult> for Part {
    fn from(tool_result: forge_domain::ToolResult) -> Self {
        Part::FunctionResponse {
            function_response: FunctionResponseData {
                name: tool_result.name.to_string(),
                response: serde_json::to_value(&tool_result.output)
                    .unwrap_or(serde_json::json!({})),
            },
        }
    }
}

impl From<forge_domain::Image> for Part {
    fn from(image: forge_domain::Image) -> Self {
        Part::Image {
            inline_data: ImageSource {
                mime_type: Some(image.mime_type().to_string()),
                data: Some(image.data().to_string()),
            },
            cache_control: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{ToolCallArguments, ToolCallFull, ToolCallId, ToolName, ToolResult};

    use super::*;

    #[test]
    fn test_tool_call_args_serialization() {
        // Create a ToolCallFull with Unparsed JSON arguments (as it would come from
        // API)
        let tool_call = ToolCallFull {
            name: ToolName::new("patch"),
            call_id: None,
            arguments: ToolCallArguments::from_json(
                r#"{"file_path":"test.rs","old_string":"foo","new_string":"bar"}"#,
            ),
            thought_signature: None,
        };

        // Convert to Google Part
        let part = Part::from(tool_call);

        // Verify it serializes correctly
        let serialized = serde_json::to_value(&part).unwrap();

        // The Part enum serializes using snake_case for the variant name
        if let Some(function_call) = serialized.get("function_call") {
            if let Some(args) = function_call.get("args") {
                // Args should be a proper JSON object, not a string
                assert!(
                    args.is_object(),
                    "Args should be deserialized as JSON object, not string: {:?}",
                    args
                );

                // Verify fields are accessible
                assert_eq!(args["file_path"], "test.rs");
                assert_eq!(args["old_string"], "foo");
                assert_eq!(args["new_string"], "bar");
            } else {
                panic!("Expected function_call.args to exist");
            }
        } else {
            panic!("Expected function_call variant");
        }
    }

    #[test]
    fn test_multiple_tool_calls_serialization() {
        // Simulate multiple tool calls with different args
        let tool_calls = vec![
            ToolCallFull {
                name: ToolName::new("remove"),
                call_id: None,
                arguments: ToolCallArguments::from_json(r#"{"path":"file1.rs"}"#),
                thought_signature: None,
            },
            ToolCallFull {
                name: ToolName::new("remove"),
                call_id: None,
                arguments: ToolCallArguments::from_json(r#"{"path":"file2.rs"}"#),
                thought_signature: None,
            },
        ];

        // Convert each to Part and serialize
        for (i, tool_call) in tool_calls.into_iter().enumerate() {
            let part = Part::from(tool_call);
            let serialized = serde_json::to_value(&part).unwrap();

            if let Some(function_call) = serialized.get("function_call") {
                if let Some(args) = function_call.get("args") {
                    assert!(
                        args.is_object(),
                        "Tool call {} args should be object, got: {:?}",
                        i,
                        args
                    );
                    let expected_path = format!("file{}.rs", i + 1);
                    assert_eq!(args["path"], expected_path, "Tool call {} path mismatch", i);
                } else {
                    panic!("Expected function_call.args for tool call {}", i);
                }
            } else {
                panic!("Expected function_call variant for tool call {}", i);
            }
        }
    }

    #[test]
    fn test_consecutive_tool_results_grouped() {
        use forge_domain::{Context, ContextMessage, ModelId};

        // Create a context with multiple consecutive tool results (simulating 13 read
        // calls)
        let mut context = Context::default();

        // Add initial user message
        context = context.add_message(ContextMessage::user(
            "Read these files",
            Some(ModelId::new("test")),
        ));

        // Add assistant message with tool calls
        context = context.add_message(ContextMessage::assistant("", None, None, None));

        // Add 13 consecutive tool results (like in the dump)
        for i in 1..=13 {
            let tool_result = ToolResult::new("read")
                .call_id(ToolCallId::new(format!("call_{}", i)))
                .success(format!("Content of file {}", i));
            context = context.add_message(ContextMessage::tool_result(tool_result));
        }

        // Convert to Google Request
        let request = Request::from(context);

        // Verify structure:
        // 1. First content: user message
        // 2. Second content: assistant message (might be empty and filtered out)
        // 3. Third content: ALL 13 tool results grouped together

        // Find the content with tool results
        let tool_result_content = request
            .contents
            .iter()
            .find(|c| {
                c.parts
                    .iter()
                    .any(|p| matches!(p, Part::FunctionResponse { .. }))
            })
            .expect("Should have a content with function responses");

        // Verify all 13 tool results are in ONE Content object
        let function_response_count = tool_result_content
            .parts
            .iter()
            .filter(|p| matches!(p, Part::FunctionResponse { .. }))
            .count();

        assert_eq!(
            function_response_count, 13,
            "All 13 tool results should be grouped into a single Content with 13 FunctionResponse parts"
        );

        // Verify the role is User
        assert_eq!(
            tool_result_content.role,
            Some(Role::User),
            "Tool results should have User role"
        );
    }

    #[test]
    fn test_non_consecutive_tool_results_not_grouped() {
        use forge_domain::{Context, ContextMessage, ModelId};

        // Create a context with tool results separated by other messages
        let mut context = Context::default();

        // User message
        context = context.add_message(ContextMessage::user(
            "First task",
            Some(ModelId::new("test")),
        ));

        // Tool result #1
        let tool_result_1 = ToolResult::new("read")
            .call_id(ToolCallId::new("call_1"))
            .success("Result 1");
        context = context.add_message(ContextMessage::tool_result(tool_result_1));

        // Assistant message (breaks the sequence)
        context = context.add_message(ContextMessage::assistant(
            "Let me do more",
            None,
            None,
            None,
        ));

        // Tool result #2 (should be in a separate Content)
        let tool_result_2 = ToolResult::new("read")
            .call_id(ToolCallId::new("call_2"))
            .success("Result 2");
        context = context.add_message(ContextMessage::tool_result(tool_result_2));

        // Convert to Google Request
        let request = Request::from(context);

        // Count how many Content objects have FunctionResponse parts
        let contents_with_tool_results: Vec<_> = request
            .contents
            .iter()
            .filter(|c| {
                c.parts
                    .iter()
                    .any(|p| matches!(p, Part::FunctionResponse { .. }))
            })
            .collect();

        // Should have 2 separate Content objects for the 2 non-consecutive tool results
        assert_eq!(
            contents_with_tool_results.len(),
            2,
            "Non-consecutive tool results should be in separate Content objects"
        );

        // Each should have exactly 1 FunctionResponse
        for content in contents_with_tool_results {
            let count = content
                .parts
                .iter()
                .filter(|p| matches!(p, Part::FunctionResponse { .. }))
                .count();
            assert_eq!(
                count, 1,
                "Each separate tool result group should have 1 FunctionResponse"
            );
        }
    }

    #[test]
    fn test_tool_choice_conversion() {
        use forge_domain::ToolChoice;

        // Test Auto
        let config = ToolConfig::from(ToolChoice::Auto);
        let fc_config = config.function_calling_config.unwrap();
        assert!(matches!(fc_config.mode, Some(FunctionCallingMode::Auto)));
        assert!(fc_config.allowed_function_names.is_none());

        // Test None
        let config = ToolConfig::from(ToolChoice::None);
        let fc_config = config.function_calling_config.unwrap();
        assert!(matches!(fc_config.mode, Some(FunctionCallingMode::None)));
        assert!(fc_config.allowed_function_names.is_none());

        // Test Required
        let config = ToolConfig::from(ToolChoice::Required);
        let fc_config = config.function_calling_config.unwrap();
        assert!(matches!(fc_config.mode, Some(FunctionCallingMode::Any)));
        assert!(fc_config.allowed_function_names.is_none());

        // Test Call
        let config = ToolConfig::from(ToolChoice::Call(ToolName::new("my_tool")));
        let fc_config = config.function_calling_config.unwrap();
        assert!(matches!(fc_config.mode, Some(FunctionCallingMode::Any)));
        assert_eq!(fc_config.allowed_function_names.unwrap(), vec!["my_tool"]);
    }

    #[test]
    fn test_tool_definition_conversion() {
        use forge_domain::ToolDefinition;
        use schemars::schema_for;

        #[derive(schemars::JsonSchema)]
        struct Args {
            _arg1: String,
        }

        let tool_def = ToolDefinition {
            name: ToolName::new("test_tool"),
            description: "A test tool".to_string(),
            input_schema: schema_for!(Args),
        };

        let decl = FunctionDeclaration::from(tool_def);

        assert_eq!(decl.name, "test_tool");
        assert_eq!(decl.description.unwrap(), "A test tool");

        // Check Gemini schema sanitization
        let params = decl.parameters;
        assert!(params.is_object());
        assert!(!params.as_object().unwrap().contains_key("$schema"));
        assert!(params.as_object().unwrap().contains_key("type"));
        // additionalProperties should be removed by Gemini sanitization
        assert!(
            !params
                .as_object()
                .unwrap()
                .contains_key("additionalProperties")
        );
    }

    #[test]
    fn test_text_message_conversion() {
        use forge_domain::{Role, TextMessage};

        // User message
        let msg = TextMessage::new(Role::User, "Hello");
        let content = Content::from(msg);
        assert_eq!(content.role, Some(self::Role::User));
        assert_eq!(content.parts.len(), 1);
        match &content.parts[0] {
            Part::Text { text, .. } => assert_eq!(text, "Hello"),
            _ => panic!("Expected Text part"),
        }

        // Assistant message with thought signature
        let msg = TextMessage::assistant("Hi", None, None).thought_signature("sig");
        let content = Content::from(msg);
        assert_eq!(content.role, Some(self::Role::Model));
        match &content.parts[0] {
            Part::Text { text, thought_signature, .. } => {
                assert_eq!(text, "Hi");
                assert_eq!(thought_signature.as_deref(), Some("sig"));
            }
            _ => panic!("Expected Text part"),
        }
    }

    #[test]
    fn test_image_conversion() {
        use forge_domain::Image;

        let image = Image::new_base64("base64data".to_string(), "image/png");
        let content = Content::from(image.clone());

        assert_eq!(content.role, Some(self::Role::User));
        assert_eq!(content.parts.len(), 1);

        match &content.parts[0] {
            Part::Image { inline_data, .. } => {
                assert_eq!(inline_data.mime_type.as_deref(), Some("image/png"));
                assert_eq!(inline_data.data.as_deref(), Some("base64data"));
            }
            _ => panic!("Expected Image part"),
        }

        // Test direct Part conversion
        let part = Part::from(image);
        match part {
            Part::Image { inline_data, .. } => {
                assert_eq!(inline_data.mime_type.as_deref(), Some("image/png"));
                assert_eq!(inline_data.data.as_deref(), Some("base64data"));
            }
            _ => panic!("Expected Image part"),
        }
    }

    #[test]
    fn test_response_schema_strips_dollar_schema() {
        use forge_domain::{Context, ResponseFormat};
        use schemars::schema_for;

        #[allow(dead_code)]
        #[derive(schemars::JsonSchema)]
        struct TestResponse {
            result: String,
        }

        // Create a context with a JSON schema response format
        let schema = schema_for!(TestResponse);
        let context =
            Context::default().response_format(ResponseFormat::JsonSchema(Box::new(schema)));

        // Convert to Google Request
        let request = Request::from(context);

        // Verify generation_config has response_schema
        let generation_config = request
            .generation_config
            .expect("Should have generation_config");
        let response_schema = generation_config
            .response_schema
            .expect("Should have response_schema");

        // Verify $schema field is removed
        if let Some(obj) = response_schema.as_object() {
            assert!(
                !obj.contains_key("$schema"),
                "$schema field should be removed from response_schema"
            );

            // Verify other schema properties are still present
            assert!(
                obj.contains_key("type")
                    || obj.contains_key("properties")
                    || obj.contains_key("title"),
                "Schema should still contain other properties"
            );

            // Verify additionalProperties is also removed by Gemini sanitization
            assert!(
                !obj.contains_key("additionalProperties"),
                "additionalProperties should be removed by Gemini sanitization"
            );
        } else {
            panic!("response_schema should be an object");
        }

        // Verify response_mime_type is set to application/json
        assert_eq!(
            generation_config.response_mime_type,
            Some("application/json".to_string()),
            "response_mime_type should be set for JSON schema"
        );
    }

    #[test]
    fn test_tool_result_part_conversion() {
        use forge_domain::{ToolCallId, ToolName, ToolResult};

        let result = ToolResult::new(ToolName::new("test"))
            .call_id(ToolCallId::new("call_1"))
            .success("output");

        let part = Part::from(result);

        match part {
            Part::FunctionResponse { function_response } => {
                assert_eq!(function_response.name, "test");
                // The response should be wrapped in a JSON value
                let expected = serde_json::json!({
                    "is_error": false,
                    "values": [{"text": "output"}]
                });
                assert_eq!(function_response.response, expected);
            }
            _ => panic!("Expected FunctionResponse part"),
        }
    }
}
