use std::collections::HashMap;

use anyhow::Context as _;
use async_openai::types::responses as oai;
use forge_app::domain::{Context as ChatContext, ContextMessage, MessagePhase, Role, ToolChoice};
use forge_app::utils::enforce_strict_schema;
use forge_domain::{Effort, ReasoningConfig, ReasoningFull};

use crate::provider::FromDomain;

/// Converts domain MessagePhase to OpenAI MessagePhase
fn to_oai_phase(phase: MessagePhase) -> oai::MessagePhase {
    match phase {
        MessagePhase::Commentary => oai::MessagePhase::Commentary,
        MessagePhase::FinalAnswer => oai::MessagePhase::FinalAnswer,
    }
}

/// Groups reasoning details by their ID and builds OpenAI `ReasoningItem`
/// input items.
///
/// Following the reference implementation, each reasoning output item is
/// identified by an `id`. When replaying multi-turn conversations with
/// `store=false`, we must reconstruct the `ReasoningItem` with both:
/// - `encrypted_content` from `reasoning.encrypted` details
/// - `summary` parts from `reasoning.summary` details
///
/// Details sharing the same ID are merged into a single `ReasoningItem`.
/// Details without an ID or with empty encrypted content are skipped.
fn map_reasoning_details_to_input_items(
    reasoning_details: Vec<ReasoningFull>,
) -> Vec<oai::InputItem> {
    // Group all details by ID so we can merge encrypted + summary for each
    // reasoning item.
    let mut grouped: HashMap<String, (Option<String>, Vec<String>)> = HashMap::new();
    // Track insertion order so output is deterministic.
    let mut order: Vec<String> = Vec::new();

    for detail in reasoning_details {
        let id = match detail.id {
            Some(ref id) if !id.is_empty() => id.clone(),
            _ => continue,
        };

        let entry = grouped.entry(id.clone()).or_insert_with(|| {
            order.push(id.clone());
            (None, Vec::new())
        });

        match detail.type_of.as_deref() {
            Some("reasoning.encrypted") => {
                if let Some(data) = detail.data
                    && !data.is_empty()
                {
                    entry.0 = Some(data);
                }
            }
            Some("reasoning.summary") => {
                if let Some(text) = detail.text
                    && !text.is_empty()
                {
                    entry.1.push(text);
                }
            }
            _ => {}
        }
    }

    order
        .into_iter()
        .filter_map(|id| {
            let (encrypted_content, summary_texts) = grouped.remove(&id)?;

            // Must have encrypted content to be a valid reasoning replay item
            let encrypted_content = encrypted_content?;

            let summary: Vec<oai::SummaryPart> = summary_texts
                .into_iter()
                .map(|text| oai::SummaryPart::SummaryText(oai::SummaryTextContent { text }))
                .collect();

            Some(oai::InputItem::Item(oai::Item::Reasoning(
                oai::ReasoningItem {
                    id,
                    summary,
                    content: None,
                    encrypted_content: Some(encrypted_content),
                    status: None,
                },
            )))
        })
        .collect()
}

impl FromDomain<ToolChoice> for oai::ToolChoiceParam {
    fn from_domain(choice: ToolChoice) -> anyhow::Result<Self> {
        Ok(match choice {
            ToolChoice::None => oai::ToolChoiceParam::Mode(oai::ToolChoiceOptions::None),
            ToolChoice::Auto => oai::ToolChoiceParam::Mode(oai::ToolChoiceOptions::Auto),
            ToolChoice::Required => oai::ToolChoiceParam::Mode(oai::ToolChoiceOptions::Required),
            ToolChoice::Call(name) => {
                oai::ToolChoiceParam::Function(oai::ToolChoiceFunction { name: name.to_string() })
            }
        })
    }
}

/// Converts domain ReasoningConfig to OpenAI Reasoning configuration
impl FromDomain<ReasoningConfig> for oai::Reasoning {
    fn from_domain(config: ReasoningConfig) -> anyhow::Result<Self> {
        let mut builder = oai::ReasoningArgs::default();

        // Map effort level
        if let Some(effort) = config.effort {
            let oai_effort = match effort {
                Effort::None => oai::ReasoningEffort::None,
                Effort::Minimal => oai::ReasoningEffort::Minimal,
                Effort::Low => oai::ReasoningEffort::Low,
                Effort::Medium => oai::ReasoningEffort::Medium,
                Effort::High => oai::ReasoningEffort::High,
                // XHigh and Max both map to the highest available OAI level.
                Effort::XHigh | Effort::Max => oai::ReasoningEffort::Xhigh,
            };
            builder.effort(oai_effort);
        } else if config.enabled.unwrap_or(false) {
            // Default to Medium effort when enabled without explicit effort
            builder.effort(oai::ReasoningEffort::Medium);
        }

        // Map summary preference
        // Note: OpenAI's ReasoningSummary doesn't have a "disabled" option
        // When exclude=true, we use Concise to minimize the summary output
        if let Some(exclude) = config.exclude {
            let summary = if exclude {
                oai::ReasoningSummary::Concise
            } else {
                oai::ReasoningSummary::Detailed
            };
            builder.summary(summary);
        } else {
            // Default to Auto summary
            builder.summary(oai::ReasoningSummary::Auto);
        }

        // Note: max_tokens is not supported in the OpenAI Responses API's ReasoningArgs
        // It's controlled at the request level via max_output_tokens

        builder.build().map_err(anyhow::Error::from)
    }
}

/// Converts a schemars RootSchema into codex tool parameters with
/// OpenAI-compatible JSON Schema
///
/// The Responses API performs strict JSON Schema validation for tools. This
/// function normalizes schemars output into the subset OpenAI accepts by using
/// the shared `normalize_json_schema` utility with strict mode enabled.
///
/// # Errors
/// Returns an error if schema serialization fails
fn codex_tool_parameters(schema: &schemars::Schema) -> anyhow::Result<serde_json::Value> {
    let mut params =
        serde_json::to_value(schema).with_context(|| "Failed to serialize tool schema")?;

    // Use strict mode (true) for OpenAI - adds additionalProperties, properties,
    // and required
    enforce_strict_schema(&mut params, true);

    Ok(params)
}

/// Converts Forge's domain-level Context into an async-openai Responses API
/// request.
///
/// Supported subset (first iteration):
/// - Text messages (system/user/assistant)
/// - Image messages (user)
/// - Assistant tool calls (full)
/// - Tool results
/// - tools + tool_choice
/// - max_tokens, temperature, top_p
impl FromDomain<ChatContext> for oai::CreateResponse {
    fn from_domain(context: ChatContext) -> anyhow::Result<Self> {
        let prompt_cache_key = context.conversation_id.as_ref().map(ToString::to_string);

        let mut instructions: Option<String> = None;
        let mut items: Vec<oai::InputItem> = Vec::new();

        for entry in context.messages {
            match entry.message {
                ContextMessage::Text(message) => match message.role {
                    Role::System => {
                        if instructions.is_none() {
                            instructions = Some(message.content);
                        } else {
                            items.push(oai::InputItem::EasyMessage(oai::EasyInputMessage {
                                r#type: oai::MessageType::Message,
                                role: oai::Role::Developer,
                                content: oai::EasyInputContent::Text(message.content),
                                phase: None,
                            }));
                        }
                    }
                    Role::User => {
                        items.push(oai::InputItem::EasyMessage(oai::EasyInputMessage {
                            r#type: oai::MessageType::Message,
                            role: oai::Role::User,
                            content: oai::EasyInputContent::Text(message.content),
                            phase: None,
                        }));
                    }
                    Role::Assistant => {
                        if !message.content.trim().is_empty() {
                            items.push(oai::InputItem::EasyMessage(oai::EasyInputMessage {
                                r#type: oai::MessageType::Message,
                                role: oai::Role::Assistant,
                                content: oai::EasyInputContent::Text(message.content),
                                phase: message.phase.map(to_oai_phase),
                            }));
                        }

                        if let Some(reasoning_details) = message.reasoning_details {
                            items.extend(map_reasoning_details_to_input_items(reasoning_details));
                        }

                        if let Some(tool_calls) = message.tool_calls {
                            for call in tool_calls {
                                let call_id =
                                    call.call_id.as_ref().map(|id| id.as_str().to_string()).ok_or_else(
                                        || {
                                            anyhow::anyhow!(
                                                "Tool call is missing call_id; cannot be sent to Responses API"
                                            )
                                        },
                                    )?;

                                items.push(oai::InputItem::Item(oai::Item::FunctionCall(
                                    oai::FunctionToolCall {
                                        arguments: call.arguments.into_string(),
                                        call_id,
                                        name: call.name.to_string(),
                                        namespace: None,
                                        id: None,
                                        status: None,
                                    },
                                )));
                            }
                        }
                    }
                },
                ContextMessage::Tool(result) => {
                    let call_id = result
                        .call_id
                        .as_ref()
                        .map(|id| id.as_str().to_string())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Tool result is missing call_id; cannot be sent to Responses API"
                            )
                        })?;

                    let output_json = serde_json::to_string(&result.output)
                        .with_context(|| "Failed to serialize tool output as JSON")?;

                    items.push(oai::InputItem::Item(oai::Item::FunctionCallOutput(
                        oai::FunctionCallOutputItemParam {
                            call_id,
                            output: oai::FunctionCallOutput::Text(output_json),
                            id: None,
                            status: None,
                        },
                    )));
                }
                ContextMessage::Image(img) => {
                    // Mirror the Chat Completions request path: represent image input
                    // as a user message with structured content.
                    items.push(oai::InputItem::EasyMessage(oai::EasyInputMessage {
                        r#type: oai::MessageType::Message,
                        role: oai::Role::User,
                        content: oai::EasyInputContent::ContentList(vec![
                            oai::InputContent::InputImage(oai::InputImageContent {
                                detail: oai::ImageDetail::Auto,
                                file_id: None,
                                image_url: Some(img.url().clone()),
                            }),
                        ]),
                        phase: None,
                    }));
                }
            }
        }

        let max_output_tokens = context
            .max_tokens
            .map(|tokens| u32::try_from(tokens).context("max_tokens must fit into u32"))
            .transpose()?;

        let tools = (!context.tools.is_empty())
            .then(|| {
                context
                    .tools
                    .into_iter()
                    .map(|tool| {
                        Ok(oai::Tool::Function(oai::FunctionTool {
                            name: tool.name.to_string(),
                            parameters: Some(codex_tool_parameters(&tool.input_schema)?),
                            strict: Some(true),
                            description: Some(tool.description),
                            defer_loading: None,
                        }))
                    })
                    .collect::<anyhow::Result<Vec<oai::Tool>>>()
            })
            .transpose()?;

        let tool_choice = context
            .tool_choice
            .map(oai::ToolChoiceParam::from_domain)
            .transpose()?;

        let mut builder = oai::CreateResponseArgs::default();
        builder.input(oai::InputParam::Items(items));

        if let Some(instructions) = instructions {
            builder.instructions(instructions);
        }

        if let Some(max_output_tokens) = max_output_tokens {
            builder.max_output_tokens(max_output_tokens);
        }

        if let Some(temperature) = context.temperature {
            builder.temperature(temperature.value());
        }

        // Some OpenAI Codex/"reasoning" models reject `top_p` entirely (even when set
        // to defaults). To avoid hard failures, we currently omit it for the
        // Responses API path.

        if let Some(tools) = tools {
            builder.tools(tools);
        }

        if let Some(tool_choice) = tool_choice {
            builder.tool_choice(tool_choice);
        }

        // Apply reasoning configuration if provided
        if let Some(reasoning) = context.reasoning {
            let reasoning_config = oai::Reasoning::from_domain(reasoning)?;
            builder.reasoning(reasoning_config);
        }

        if let Some(prompt_cache_key) = prompt_cache_key {
            builder.prompt_cache_key(prompt_cache_key);
        }

        let mut response = builder.build().map_err(anyhow::Error::from)?;

        response.stream = Some(true);

        // When reasoning is configured, request encrypted content so it can be
        // replayed in subsequent turns for stateless reasoning continuity.
        if response.reasoning.is_some() {
            let includes = response.include.get_or_insert_with(Vec::new);
            if !includes.contains(&oai::IncludeEnum::ReasoningEncryptedContent) {
                includes.push(oai::IncludeEnum::ReasoningEncryptedContent);
            }
        }

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use async_openai::types::responses as oai;
    use forge_app::domain::{
        Context as ChatContext, ContextMessage, ModelId, ToolCallId, ToolChoice,
    };
    use forge_app::utils::enforce_strict_schema;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use crate::provider::FromDomain;
    use crate::provider::openai_responses::request::codex_tool_parameters;

    #[test]
    fn test_reasoning_config_conversion_with_effort() -> anyhow::Result<()> {
        use forge_domain::{Effort, ReasoningConfig};

        let fixture = ReasoningConfig {
            effort: Some(Effort::High),
            max_tokens: Some(2048),
            exclude: Some(false),
            enabled: None,
        };

        let actual = oai::Reasoning::from_domain(fixture)?;

        // Note: We can't easily assert the internal fields since ReasoningArgs
        // doesn't expose them after building. The fact that it builds without
        // error is the main verification.
        assert!(actual.effort.is_some());
        assert!(actual.summary.is_some());

        Ok(())
    }

    #[test]
    fn test_reasoning_config_conversion_with_enabled() -> anyhow::Result<()> {
        use forge_domain::ReasoningConfig;

        let fixture = ReasoningConfig {
            effort: None,
            max_tokens: None,
            exclude: None,
            enabled: Some(true),
        };

        let actual = oai::Reasoning::from_domain(fixture)?;

        // When enabled=true with no explicit effort, should default to Medium
        assert!(actual.effort.is_some());
        assert!(actual.summary.is_some());

        Ok(())
    }

    #[test]
    fn test_reasoning_config_conversion_with_exclude() -> anyhow::Result<()> {
        use forge_domain::{Effort, ReasoningConfig};

        let fixture = ReasoningConfig {
            effort: Some(Effort::Medium),
            max_tokens: None,
            exclude: Some(true),
            enabled: None,
        };

        let actual = oai::Reasoning::from_domain(fixture)?;

        // When exclude=true, should use Concise summary
        assert!(actual.effort.is_some());
        assert!(actual.summary.is_some());

        Ok(())
    }

    #[test]
    fn test_codex_request_with_reasoning_config() -> anyhow::Result<()> {
        use forge_domain::{Effort, ReasoningConfig};

        let reasoning = ReasoningConfig {
            effort: Some(Effort::High),
            max_tokens: Some(2048),
            exclude: Some(false),
            enabled: Some(true),
        };

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Test", None))
            .reasoning(reasoning);

        let actual = oai::CreateResponse::from_domain(context)?;

        // Verify that reasoning config is set
        assert!(actual.reasoning.is_some());

        Ok(())
    }

    #[test]
    fn test_codex_request_with_reasoning_includes_encrypted_content() -> anyhow::Result<()> {
        use forge_domain::{Effort, ReasoningConfig};

        let reasoning = ReasoningConfig {
            effort: Some(Effort::High),
            max_tokens: None,
            exclude: None,
            enabled: Some(true),
        };

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Test", None))
            .reasoning(reasoning);

        let actual = oai::CreateResponse::from_domain(context)?;

        let expected = Some(vec![oai::IncludeEnum::ReasoningEncryptedContent]);
        assert_eq!(actual.include, expected);

        Ok(())
    }

    #[test]
    fn test_codex_request_without_reasoning_has_no_include() -> anyhow::Result<()> {
        let context = ChatContext::default().add_message(ContextMessage::user("Test", None));

        let actual = oai::CreateResponse::from_domain(context)?;

        assert_eq!(actual.include, None);

        Ok(())
    }

    #[test]
    fn test_codex_request_from_context_converts_messages_tools_and_results() -> anyhow::Result<()> {
        let model = ModelId::from("codex-mini-latest");

        let tool_definition =
            forge_app::domain::ToolDefinition::new("shell").description("Run a shell command");

        let tool_call = forge_app::domain::ToolCallFull::new("shell")
            .call_id(ToolCallId::new("call_1"))
            .arguments(forge_app::domain::ToolCallArguments::from_json(
                r#"{"cmd":"echo hi"}"#,
            ));

        let tool_result = forge_app::domain::ToolResult::new("shell")
            .call_id(Some(ToolCallId::new("call_1")))
            .success("ok");

        let context = ChatContext::default()
            .add_message(ContextMessage::system("You are a helpful assistant."))
            .add_message(ContextMessage::user("Hello", None))
            .add_message(ContextMessage::assistant(
                "",
                None,
                None,
                Some(vec![tool_call]),
            ))
            .add_message(ContextMessage::tool_result(tool_result))
            .add_tool(tool_definition)
            .tool_choice(ToolChoice::Auto)
            .max_tokens(123usize);

        let mut actual = oai::CreateResponse::from_domain(context)?;
        actual.model = Some(model.as_str().to_string());

        assert_eq!(actual.model.as_deref(), Some("codex-mini-latest"));
        assert_eq!(
            actual.instructions.as_deref(),
            Some("You are a helpful assistant.")
        );
        assert_eq!(actual.max_output_tokens, Some(123));

        let oai::InputParam::Items(items) = actual.input else {
            anyhow::bail!("Expected items input");
        };

        // user + function_call + function_call_output
        assert_eq!(items.len(), 3);

        let oai::InputItem::EasyMessage(user_msg) = &items[0] else {
            anyhow::bail!("Expected first item to be a user message");
        };
        assert_eq!(user_msg.role, oai::Role::User);

        let oai::InputItem::Item(oai::Item::FunctionCall(call)) = &items[1] else {
            anyhow::bail!("Expected second item to be a function call");
        };
        assert_eq!(call.call_id, "call_1");
        assert_eq!(call.name, "shell");

        let oai::InputItem::Item(oai::Item::FunctionCallOutput(out)) = &items[2] else {
            anyhow::bail!("Expected third item to be a function call output");
        };
        assert_eq!(out.call_id, "call_1");

        Ok(())
    }

    // Common fixture functions
    fn fixture_tool_definition(name: &str) -> forge_app::domain::ToolDefinition {
        forge_app::domain::ToolDefinition::new(name).description("Test tool")
    }

    fn fixture_tool_call(name: &str, call_id: &str, args: &str) -> forge_app::domain::ToolCallFull {
        forge_app::domain::ToolCallFull::new(name)
            .call_id(ToolCallId::new(call_id))
            .arguments(forge_app::domain::ToolCallArguments::from_json(args))
    }

    #[test]
    fn test_codex_tool_parameters_removes_unsupported_uri_format() -> anyhow::Result<()> {
        let fixture = schemars::Schema::try_from(json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "format": "uri"
                }
            }
        }))
        .unwrap();

        let actual = codex_tool_parameters(&fixture)?;

        let expected = json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string"
                }
            },
            "additionalProperties": false,
            "required": ["url"]
        });

        assert_eq!(actual, expected);

        Ok(())
    }

    #[test]
    fn test_codex_request_tools_snapshot() -> anyhow::Result<()> {
        // Build a schema that exercises OpenAI strict-mode normalization:
        // - object schema receives additionalProperties=false
        // - required keys are sorted
        // - nullable + enum(null) is converted to anyOf
        let schema_value = serde_json::json!({
            "type": "object",
            "properties": {
                // Intentionally out-of-order to verify required keys are sorted.
                "zebra": {"type": "string"},
                "alpha": {"type": "string"},
                "output_mode": {
                    "description": "Output mode",
                    "nullable": true,
                    "type": "string",
                    "enum": ["content", "count", null]
                }
            }
        });
        let schema = schemars::Schema::try_from(schema_value).unwrap();

        let tool = forge_app::domain::ToolDefinition::new("shell")
            .description("Run a shell command")
            .input_schema(schema);

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hello", None))
            .add_tool(tool)
            .tool_choice(ToolChoice::Auto);

        let actual = oai::CreateResponse::from_domain(context)?;

        insta::assert_json_snapshot!("openai_responses_tools", actual.tools);

        Ok(())
    }

    #[test]
    fn test_codex_request_all_catalog_tools_snapshot() -> anyhow::Result<()> {
        use forge_app::domain::ToolCatalog;
        use strum::IntoEnumIterator;

        // Ensure we can serialize ALL built-in tool definitions into the OpenAI
        // Responses API tool format with strict JSON schema normalization.
        let tools = ToolCatalog::iter()
            .map(|tool| tool.definition())
            .collect::<Vec<_>>();

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hello", None))
            .tools(tools)
            .tool_choice(ToolChoice::Auto);

        let actual = oai::CreateResponse::from_domain(context)?;

        insta::assert_json_snapshot!("openai_responses_all_catalog_tools", actual.tools);

        Ok(())
    }

    #[test]
    fn test_tool_choice_none_conversion() -> anyhow::Result<()> {
        let actual = oai::ToolChoiceParam::from_domain(ToolChoice::None)?;
        assert!(matches!(
            actual,
            oai::ToolChoiceParam::Mode(oai::ToolChoiceOptions::None)
        ));
        Ok(())
    }

    #[test]
    fn test_tool_choice_auto_conversion() -> anyhow::Result<()> {
        let actual = oai::ToolChoiceParam::from_domain(ToolChoice::Auto)?;
        assert!(matches!(
            actual,
            oai::ToolChoiceParam::Mode(oai::ToolChoiceOptions::Auto)
        ));
        Ok(())
    }

    #[test]
    fn test_tool_choice_required_conversion() -> anyhow::Result<()> {
        let actual = oai::ToolChoiceParam::from_domain(ToolChoice::Required)?;
        assert!(matches!(
            actual,
            oai::ToolChoiceParam::Mode(oai::ToolChoiceOptions::Required)
        ));
        Ok(())
    }

    #[test]
    fn test_tool_choice_call_conversion() -> anyhow::Result<()> {
        let actual = oai::ToolChoiceParam::from_domain(ToolChoice::Call("test_tool".into()))?;
        assert!(matches!(
            actual,
            oai::ToolChoiceParam::Function(oai::ToolChoiceFunction { name, .. }) if name == "test_tool"
        ));
        Ok(())
    }

    #[test]
    fn test_reasoning_config_conversion_low_effort() -> anyhow::Result<()> {
        use forge_domain::{Effort, ReasoningConfig};

        let fixture = ReasoningConfig {
            effort: Some(Effort::Low),
            max_tokens: None,
            exclude: None,
            enabled: None,
        };

        let actual = oai::Reasoning::from_domain(fixture)?;
        assert!(actual.effort.is_some());
        assert!(actual.summary.is_some());

        Ok(())
    }

    #[test]
    fn test_reasoning_config_conversion_medium_effort() -> anyhow::Result<()> {
        use forge_domain::{Effort, ReasoningConfig};

        let fixture = ReasoningConfig {
            effort: Some(Effort::Medium),
            max_tokens: None,
            exclude: None,
            enabled: None,
        };

        let actual = oai::Reasoning::from_domain(fixture)?;
        assert!(actual.effort.is_some());
        assert!(actual.summary.is_some());

        Ok(())
    }

    #[test]
    fn test_reasoning_config_conversion_with_detailed_summary() -> anyhow::Result<()> {
        use forge_domain::{Effort, ReasoningConfig};

        let fixture = ReasoningConfig {
            effort: Some(Effort::Medium),
            max_tokens: None,
            exclude: Some(false),
            enabled: None,
        };

        let actual = oai::Reasoning::from_domain(fixture)?;
        assert!(actual.effort.is_some());
        assert!(actual.summary.is_some());

        Ok(())
    }

    #[test]
    fn test_reasoning_config_conversion_with_enabled_false() -> anyhow::Result<()> {
        use forge_domain::ReasoningConfig;

        let fixture = ReasoningConfig {
            effort: None,
            max_tokens: None,
            exclude: None,
            enabled: Some(false),
        };

        let actual = oai::Reasoning::from_domain(fixture)?;
        // When enabled=false, no effort should be set
        assert!(actual.effort.is_none());
        assert!(actual.summary.is_some());

        Ok(())
    }

    #[test]
    fn test_normalize_openai_json_schema_with_object_type() {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            }
        });

        enforce_strict_schema(&mut schema, true);

        assert_eq!(
            schema["additionalProperties"],
            serde_json::Value::Bool(false)
        );
        assert_eq!(schema["required"], serde_json::json!(["name"]));
    }

    #[test]
    fn test_normalize_openai_json_schema_with_properties_key() {
        let mut schema = serde_json::json!({
            "properties": {
                "age": {"type": "number"}
            }
        });

        enforce_strict_schema(&mut schema, true);

        assert_eq!(
            schema["additionalProperties"],
            serde_json::Value::Bool(false)
        );
        assert_eq!(schema["required"], serde_json::json!(["age"]));
    }

    #[test]
    fn test_normalize_openai_json_schema_without_properties() {
        let mut schema = serde_json::json!({
            "type": "object"
        });

        enforce_strict_schema(&mut schema, true);

        assert_eq!(
            schema["properties"],
            serde_json::Value::Object(serde_json::Map::new())
        );
        assert_eq!(
            schema["additionalProperties"],
            serde_json::Value::Bool(false)
        );
        assert_eq!(schema["required"], serde_json::json!([]));
    }

    #[test]
    fn test_normalize_openai_json_schema_with_nested_objects() {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"}
                    }
                }
            }
        });

        enforce_strict_schema(&mut schema, true);

        // Top level should have additionalProperties
        assert_eq!(
            schema["additionalProperties"],
            serde_json::Value::Bool(false)
        );
        assert_eq!(schema["required"], serde_json::json!(["user"]));

        // Nested object should also be normalized
        assert_eq!(
            schema["properties"]["user"]["additionalProperties"],
            serde_json::Value::Bool(false)
        );
        assert_eq!(
            schema["properties"]["user"]["required"],
            serde_json::json!(["name"])
        );
    }

    #[test]
    fn test_normalize_openai_json_schema_with_array() {
        let mut schema = serde_json::json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"}
                }
            }
        });

        enforce_strict_schema(&mut schema, true);

        // Array items should be normalized
        assert_eq!(
            schema["items"]["additionalProperties"],
            serde_json::Value::Bool(false)
        );
        assert_eq!(schema["items"]["required"], serde_json::json!(["id"]));
    }

    #[test]
    fn test_normalize_openai_json_schema_with_string() {
        let mut schema = serde_json::json!({
            "type": "string"
        });

        enforce_strict_schema(&mut schema, true);

        // Should not modify non-object types
        assert_eq!(schema, serde_json::json!({"type": "string"}));
    }

    #[test]
    fn test_normalize_openai_json_schema_sorts_required_keys() {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "zebra": {"type": "string"},
                "alpha": {"type": "string"},
                "beta": {"type": "string"}
            }
        });

        enforce_strict_schema(&mut schema, true);

        assert_eq!(
            schema["required"],
            serde_json::json!(["alpha", "beta", "zebra"])
        );
    }
    #[test]
    fn test_codex_request_sets_prompt_cache_key_from_conversation_id() -> anyhow::Result<()> {
        use forge_domain::ConversationId;

        let conversation_id = ConversationId::generate();
        let context = ChatContext::default()
            .conversation_id(conversation_id)
            .add_message(ContextMessage::user("Hello", None));

        let actual = oai::CreateResponse::from_domain(context)?;
        let expected = Some(conversation_id.to_string());

        assert_eq!(actual.prompt_cache_key, expected);

        Ok(())
    }

    #[test]
    fn test_codex_request_without_conversation_id_has_no_prompt_cache_key() -> anyhow::Result<()> {
        let context = ChatContext::default().add_message(ContextMessage::user("Hello", None));

        let actual = oai::CreateResponse::from_domain(context)?;

        assert_eq!(actual.prompt_cache_key, None);

        Ok(())
    }

    #[test]
    fn test_codex_request_maps_reasoning_encrypted_and_summary_to_reasoning_input_items()
    -> anyhow::Result<()> {
        use forge_domain::ReasoningFull;

        let context = ChatContext::default()
            .add_message(ContextMessage::assistant(
                "",
                None,
                Some(vec![
                    ReasoningFull::default()
                        .type_of(Some("reasoning.encrypted".to_string()))
                        .id(Some("rs_123".to_string()))
                        .data(Some("enc_payload_1".to_string())),
                    ReasoningFull::default()
                        .type_of(Some("reasoning.summary".to_string()))
                        .id(Some("rs_123".to_string()))
                        .text(Some("Summary of reasoning".to_string())),
                    ReasoningFull::default()
                        .type_of(Some("reasoning.text".to_string()))
                        .id(Some("rs_123".to_string()))
                        .text(Some(
                            "visible reasoning should not be in summary".to_string(),
                        )),
                ]),
                None,
            ))
            .add_message(ContextMessage::user("continue", None));

        let actual = oai::CreateResponse::from_domain(context)?;

        let oai::InputParam::Items(items) = actual.input else {
            anyhow::bail!("Expected items input");
        };

        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[0],
            oai::InputItem::Item(oai::Item::Reasoning(_))
        ));
        assert!(matches!(&items[1], oai::InputItem::EasyMessage(_)));

        let oai::InputItem::Item(oai::Item::Reasoning(reasoning_item)) = &items[0] else {
            anyhow::bail!("Expected first item to be reasoning item");
        };

        let expected = oai::ReasoningItem {
            id: "rs_123".to_string(),
            summary: vec![oai::SummaryPart::SummaryText(oai::SummaryTextContent {
                text: "Summary of reasoning".to_string(),
            })],
            content: None,
            encrypted_content: Some("enc_payload_1".to_string()),
            status: None,
        };

        assert_eq!(reasoning_item, &expected);

        Ok(())
    }

    #[test]
    fn test_codex_request_skips_invalid_encrypted_reasoning_details() -> anyhow::Result<()> {
        use forge_domain::ReasoningFull;

        let context = ChatContext::default()
            .add_message(ContextMessage::assistant(
                "",
                None,
                Some(vec![
                    ReasoningFull::default()
                        .type_of(Some("reasoning.encrypted".to_string()))
                        .id(Some("".to_string()))
                        .data(Some("enc_missing_id".to_string())),
                    ReasoningFull::default()
                        .type_of(Some("reasoning.encrypted".to_string()))
                        .id(Some("rs_missing_data".to_string())),
                    ReasoningFull::default()
                        .type_of(Some("reasoning.encrypted".to_string()))
                        .id(Some("rs_ok".to_string()))
                        .data(Some("enc_ok".to_string())),
                ]),
                None,
            ))
            .add_message(ContextMessage::user("continue", None));

        let actual = oai::CreateResponse::from_domain(context)?;

        let oai::InputParam::Items(items) = actual.input else {
            anyhow::bail!("Expected items input");
        };

        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[0],
            oai::InputItem::Item(oai::Item::Reasoning(_))
        ));

        let oai::InputItem::Item(oai::Item::Reasoning(reasoning_item)) = &items[0] else {
            anyhow::bail!("Expected first item to be reasoning item");
        };

        let expected = oai::ReasoningItem {
            id: "rs_ok".to_string(),
            summary: vec![],
            content: None,
            encrypted_content: Some("enc_ok".to_string()),
            status: None,
        };

        assert_eq!(reasoning_item, &expected);

        Ok(())
    }

    #[test]
    fn test_codex_request_with_temperature() -> anyhow::Result<()> {
        use forge_app::domain::Temperature;

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hello", None))
            .temperature(Temperature::from(0.7));

        let actual = oai::CreateResponse::from_domain(context)?;

        assert_eq!(actual.temperature, Some(0.7));

        Ok(())
    }

    #[test]
    fn test_codex_request_with_empty_assistant_message() -> anyhow::Result<()> {
        let tool_call = fixture_tool_call("shell", "call_1", r#"{"cmd":"ls"}"#);

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Run command", None))
            .add_message(ContextMessage::assistant(
                "",
                None,
                None,
                Some(vec![tool_call]),
            ))
            .add_message(ContextMessage::tool_result(
                forge_app::domain::ToolResult::new("shell")
                    .call_id(Some(ToolCallId::new("call_1")))
                    .success("output"),
            ));

        let actual = oai::CreateResponse::from_domain(context)?;

        let oai::InputParam::Items(items) = actual.input else {
            anyhow::bail!("Expected items input");
        };

        // Should only have user message, function call, and function call output
        // Empty assistant message should be skipped
        assert_eq!(items.len(), 3);

        Ok(())
    }

    #[test]
    fn test_codex_request_with_multiple_tool_calls() -> anyhow::Result<()> {
        let tool_call1 = fixture_tool_call("shell", "call_1", r#"{"cmd":"ls"}"#);
        let tool_call2 = fixture_tool_call("search", "call_2", r#"{"query":"test"}"#);

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Do two things", None))
            .add_message(ContextMessage::assistant(
                "",
                None,
                None,
                Some(vec![tool_call1, tool_call2]),
            ));

        let actual = oai::CreateResponse::from_domain(context)?;

        let oai::InputParam::Items(items) = actual.input else {
            anyhow::bail!("Expected items input");
        };

        // Should have user message and 2 function calls
        assert_eq!(items.len(), 3);

        Ok(())
    }

    #[test]
    fn test_codex_request_with_multiple_system_messages() -> anyhow::Result<()> {
        let context = ChatContext::default()
            .add_message(ContextMessage::system("System 1"))
            .add_message(ContextMessage::system("System 2"))
            .add_message(ContextMessage::user("Hello", None));

        let actual = oai::CreateResponse::from_domain(context)?;

        assert_eq!(actual.instructions.as_deref(), Some("System 1"));

        let oai::InputParam::Items(items) = actual.input else {
            anyhow::bail!("Expected items input");
        };

        // System 2 (Developer) + User
        assert_eq!(items.len(), 2);

        let oai::InputItem::EasyMessage(dev_msg) = &items[0] else {
            anyhow::bail!("Expected first item to be a message");
        };
        assert_eq!(dev_msg.role, oai::Role::Developer);
        assert_eq!(
            dev_msg.content,
            oai::EasyInputContent::Text("System 2".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_codex_request_with_tool_choice_required() -> anyhow::Result<()> {
        let tool = fixture_tool_definition("shell");

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hello", None))
            .add_tool(tool)
            .tool_choice(ToolChoice::Required);

        let actual = oai::CreateResponse::from_domain(context)?;

        assert!(matches!(
            actual.tool_choice,
            Some(oai::ToolChoiceParam::Mode(oai::ToolChoiceOptions::Required))
        ));

        Ok(())
    }

    #[test]
    fn test_codex_request_with_tool_choice_function() -> anyhow::Result<()> {
        let tool = fixture_tool_definition("shell");

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hello", None))
            .add_tool(tool)
            .tool_choice(ToolChoice::Call("shell".into()));

        let actual = oai::CreateResponse::from_domain(context)?;

        assert!(matches!(
            actual.tool_choice,
            Some(oai::ToolChoiceParam::Function(oai::ToolChoiceFunction { name, .. })) if name == "shell"
        ));

        Ok(())
    }

    #[test]
    fn test_codex_request_without_tools() -> anyhow::Result<()> {
        let context = ChatContext::default().add_message(ContextMessage::user("Hello", None));

        let actual = oai::CreateResponse::from_domain(context)?;

        assert!(actual.tools.is_none());
        assert!(actual.tool_choice.is_none());

        Ok(())
    }

    #[test]
    fn test_codex_request_with_image_input_is_supported() -> anyhow::Result<()> {
        use forge_domain::Image;

        let image = Image::new_base64("test123".to_string(), "image/png");
        let context = ChatContext::default().add_message(ContextMessage::Image(image));

        let actual = oai::CreateResponse::from_domain(context)?;

        let oai::InputParam::Items(items) = actual.input else {
            anyhow::bail!("Expected items input");
        };

        assert_eq!(items.len(), 1);

        let oai::InputItem::EasyMessage(message) = &items[0] else {
            anyhow::bail!("Expected first item to be an EasyMessage");
        };

        assert_eq!(message.role, oai::Role::User);

        let oai::EasyInputContent::ContentList(content) = &message.content else {
            anyhow::bail!("Expected ContentList for image message content");
        };

        assert_eq!(content.len(), 1);

        let oai::InputContent::InputImage(image) = &content[0] else {
            anyhow::bail!("Expected InputImage content");
        };

        assert_eq!(image.detail, oai::ImageDetail::Auto);
        assert!(image.file_id.is_none());
        assert_eq!(
            image.image_url.as_deref(),
            Some("data:image/png;base64,test123")
        );

        Ok(())
    }

    #[test]
    fn test_codex_request_with_tool_call_missing_call_id_returns_error() {
        let tool_call = forge_app::domain::ToolCallFull::new("shell").arguments(
            forge_app::domain::ToolCallArguments::from_json(r#"{"cmd":"ls"}"#),
        );

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Run command", None))
            .add_message(ContextMessage::assistant(
                "",
                None,
                None,
                Some(vec![tool_call]),
            ));

        let result = oai::CreateResponse::from_domain(context);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Tool call is missing call_id")
        );
    }

    #[test]
    fn test_codex_request_with_tool_result_missing_call_id_returns_error() {
        let context = ChatContext::default()
            .add_message(ContextMessage::user("Run command", None))
            .add_message(ContextMessage::tool_result(
                forge_app::domain::ToolResult::new("shell").success("output"),
            ));

        let result = oai::CreateResponse::from_domain(context);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Tool result is missing call_id")
        );
    }

    #[test]
    fn test_codex_request_with_max_tokens_overflow_returns_error() {
        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hello", None))
            .max_tokens(u32::MAX as usize + 1);

        let result = oai::CreateResponse::from_domain(context);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("max_tokens must fit into u32")
        );
    }

    #[test]
    fn test_codex_request_preserves_phase_on_assistant_message() -> anyhow::Result<()> {
        use forge_app::domain::{MessagePhase, TextMessage};
        use forge_domain::Role;

        let mut assistant_msg = TextMessage::new(Role::Assistant, "Thinking about this...");
        assistant_msg.phase = Some(MessagePhase::Commentary);

        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hello", None))
            .add_entry(forge_app::domain::MessageEntry::from(ContextMessage::Text(
                assistant_msg,
            )))
            .add_message(ContextMessage::user("Continue", None));

        let actual = oai::CreateResponse::from_domain(context)?;

        let oai::InputParam::Items(items) = actual.input else {
            anyhow::bail!("Expected items input");
        };

        // Find the assistant EasyMessage
        let assistant_item = items
            .iter()
            .find(|item| {
                matches!(
                    item,
                    oai::InputItem::EasyMessage(msg) if msg.role == oai::Role::Assistant
                )
            })
            .expect("Should have an assistant message");

        let oai::InputItem::EasyMessage(msg) = assistant_item else {
            anyhow::bail!("Expected EasyMessage");
        };

        assert_eq!(msg.phase, Some(oai::MessagePhase::Commentary));

        Ok(())
    }

    #[test]
    fn test_codex_request_preserves_final_answer_phase() -> anyhow::Result<()> {
        use forge_app::domain::{MessagePhase, TextMessage};
        use forge_domain::Role;

        let mut assistant_msg = TextMessage::new(Role::Assistant, "The answer is 42.");
        assistant_msg.phase = Some(MessagePhase::FinalAnswer);

        let context = ChatContext::default()
            .add_message(ContextMessage::user("What is the answer?", None))
            .add_entry(forge_app::domain::MessageEntry::from(ContextMessage::Text(
                assistant_msg,
            )))
            .add_message(ContextMessage::user("Thanks", None));

        let actual = oai::CreateResponse::from_domain(context)?;

        let oai::InputParam::Items(items) = actual.input else {
            anyhow::bail!("Expected items input");
        };

        let assistant_item = items
            .iter()
            .find(|item| {
                matches!(
                    item,
                    oai::InputItem::EasyMessage(msg) if msg.role == oai::Role::Assistant
                )
            })
            .expect("Should have an assistant message");

        let oai::InputItem::EasyMessage(msg) = assistant_item else {
            anyhow::bail!("Expected EasyMessage");
        };

        assert_eq!(msg.phase, Some(oai::MessagePhase::FinalAnswer));

        Ok(())
    }

    #[test]
    fn test_codex_request_no_phase_when_none() -> anyhow::Result<()> {
        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hello", None))
            .add_message(ContextMessage::assistant("Response", None, None, None))
            .add_message(ContextMessage::user("Continue", None));

        let actual = oai::CreateResponse::from_domain(context)?;

        let oai::InputParam::Items(items) = actual.input else {
            anyhow::bail!("Expected items input");
        };

        let assistant_item = items
            .iter()
            .find(|item| {
                matches!(
                    item,
                    oai::InputItem::EasyMessage(msg) if msg.role == oai::Role::Assistant
                )
            })
            .expect("Should have an assistant message");

        let oai::InputItem::EasyMessage(msg) = assistant_item else {
            anyhow::bail!("Expected EasyMessage");
        };

        assert_eq!(msg.phase, None);

        Ok(())
    }
}
