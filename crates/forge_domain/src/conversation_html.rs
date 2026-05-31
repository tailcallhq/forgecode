use forge_template::Element;
use serde_json::to_string_pretty;

use crate::context::ContextMessage;
use crate::conversation::Conversation;

/// Renders a conversation as an HTML document
///
/// Creates a complete HTML page displaying the conversation's information
/// including:
/// - Basic information (ID, title)
/// - Reasoning configuration
/// - Usage statistics (token counts and costs)
/// - Context messages with tool calls and reasoning details
/// - Available tools
/// - Error styling for tool call failures
///
/// # Arguments
///
/// * `conversation` - The conversation to render
pub fn render_conversation_html(conversation: &Conversation) -> String {
    let c_title = format!(
        "Title: {}",
        conversation
            .title
            .clone()
            .unwrap_or(conversation.id.to_string())
    );
    let html = Element::new("html")
        .attr("lang", "en")
        .append(
            Element::new("head")
                .append(Element::new("meta").attr("charset", "UTF-8"))
                .append(
                    Element::new("meta")
                        .attr("name", "viewport")
                        .attr("content", "width=device-width, initial-scale=1.0"),
                )
                .append(Element::new("title").text(&c_title))
                .append(Element::new("style").text(include_str!("conversation_style.css"))), // Includes tool-call-error styles
        )
        .append(
            Element::new("body")
                // Combined Information Table
                .append(create_info_table(conversation))
                // Conversation Context Section
                .append(create_conversation_context_section(conversation))
                // Tools Section
                .append(create_tools_section(conversation)),
        );

    format!("<!DOCTYPE html>\n{}", html.render())
}

/// Renders a conversation with related agent conversations in a single HTML
/// document
///
/// Creates a complete HTML page with the main conversation followed by
/// related agent conversations. Uses anchor links for navigation.
///
/// # Arguments
///
/// * `conversation` - The main conversation to render
/// * `related` - Related agent conversations to include
pub fn render_conversation_html_with_related(
    conversation: &Conversation,
    related: &[Conversation],
) -> String {
    let c_title = format!(
        "Title: {}",
        conversation
            .title
            .clone()
            .unwrap_or(conversation.id.to_string())
    );

    let mut body = Element::new("body")
        // Combined Information Table
        .append(create_info_table(conversation))
        // Conversation Context Section
        .append(create_conversation_context_section(conversation))
        // Tools Section
        .append(create_tools_section(conversation));

    // Add related conversations section
    if !related.is_empty() {
        body = body.append(Element::new("div.section").append(
            Element::new("h2").text(format!("Related Agent Conversations ({})", related.len())),
        ));

        for related_conv in related {
            let anchor_id = format!("conversation-{}", related_conv.id);
            body = body.append(
                Element::new("div.related-conversation")
                    .attr("id", &anchor_id)
                    .append(
                        Element::new("div.back-to-main").append(
                            Element::new("a")
                                .attr("href", "#")
                                .text("⬆ Back to main conversation"),
                        ),
                    )
                    .append(create_info_table(related_conv))
                    .append(create_conversation_context_section(related_conv))
                    .append(create_tools_section(related_conv)),
            );
        }
    }

    let html = Element::new("html")
        .attr("lang", "en")
        .append(
            Element::new("head")
                .append(Element::new("meta").attr("charset", "UTF-8"))
                .append(
                    Element::new("meta")
                        .attr("name", "viewport")
                        .attr("content", "width=device-width, initial-scale=1.0"),
                )
                .append(Element::new("title").text(&c_title))
                .append(Element::new("style").text(include_str!("conversation_style.css"))),
        )
        .append(body);

    format!("<!DOCTYPE html>\n{}", html.render())
}

/// Creates a table row with a label and value
fn create_table_row(label: impl Into<String>, value: impl Into<String>) -> Element {
    Element::new("tr")
        .append(Element::new("th").text(label.into()))
        .append(Element::new("td").text(value.into()))
}

/// Creates a combined information table with all conversation metadata
fn create_info_table(conversation: &Conversation) -> Element {
    let section = Element::new("div.section").append(Element::new("h2").text("Conversation"));

    let mut table = Element::new("table")
        .append(create_table_row("ID", conversation.id.to_string()))
        .append(create_table_row(
            "Title",
            conversation
                .title
                .clone()
                .unwrap_or_else(|| "No title".to_string()),
        ));

    // Add reasoning configuration if available
    if let Some(context) = &conversation.context {
        if let Some(reasoning_config) = &context.reasoning {
            let status = match reasoning_config.enabled {
                Some(true) => "Enabled",
                Some(false) => "Disabled",
                None => "Not specified",
            };
            table = table
                .append(create_table_row("Reasoning Status", status))
                .append(create_table_row(
                    "Reasoning Effort",
                    format!("{:?}", reasoning_config.effort),
                ));

            if let Some(max_tokens) = reasoning_config.max_tokens {
                table = table.append(create_table_row(
                    "Reasoning Max Tokens",
                    format!("{max_tokens:?}"),
                ));
            }
        }

        if let Some(max_tokens) = context.max_tokens {
            table = table.append(create_table_row(
                "Max Output Tokens",
                format!("{:?}", max_tokens),
            ))
        }

        // Add usage information if available
        if let Some(usage) = context.accumulate_usage() {
            let cache_percentage = if *usage.prompt_tokens > 0 {
                (*usage.cached_tokens as f64 / *usage.prompt_tokens as f64 * 100.0) as usize
            } else {
                0
            };

            let cached_display = if cache_percentage > 0 {
                format!("{} [{}%]", usage.cached_tokens, cache_percentage)
            } else {
                format!("{}", usage.cached_tokens)
            };

            table = table
                .append(create_table_row(
                    "Input Tokens",
                    format!("{}", usage.prompt_tokens),
                ))
                .append(create_table_row("Cached Tokens", cached_display))
                .append(create_table_row(
                    "Output Tokens",
                    format!("{}", usage.completion_tokens),
                ))
                .append(create_table_row(
                    "Total Tokens",
                    format!("{}", usage.total_tokens),
                ));

            if let Some(cost) = usage.cost {
                table = table.append(create_table_row("Cost", format!("${:.4}", cost)));
            }
        }
    }

    section.append(table)
}

/// Creates a tools section displaying all available tools
fn create_tools_section(conversation: &Conversation) -> Element {
    let section = Element::new("div.section").append(Element::new("h2").text("Tools"));

    if let Some(context) = &conversation.context {
        if !context.tools.is_empty() {
            let tools_elm =
                Element::new("div.tools-section").append(context.tools.iter().map(|tool| {
                    Element::new("details.message-card.message-tool")
                        .append(
                            Element::new("summary").append(Element::span(tool.name.to_string())),
                        )
                        .append(
                            Element::new("div.main-content")
                                .append(Element::new("p").append(
                                    Element::new("strong").text(tool.description.to_string()),
                                ))
                                .append(Element::new("pre").text(
                                    to_string_pretty(&tool.input_schema).unwrap_or_default(),
                                )),
                        )
                }));
            section.append(tools_elm)
        } else {
            section.append(Element::new("p").text("No tools available"))
        }
    } else {
        section.append(Element::new("p").text("No tools available"))
    }
}

/// Creates a usage information section for a message
fn create_message_usage_section(usage: &crate::message::Usage) -> Element {
    let cache_percentage = if *usage.prompt_tokens > 0 {
        (*usage.cached_tokens as f64 / *usage.prompt_tokens as f64 * 100.0) as usize
    } else {
        0
    };

    let cached_display = if cache_percentage > 0 {
        format!("{} [{}%]", usage.cached_tokens, cache_percentage)
    } else {
        format!("{}", usage.cached_tokens)
    };

    let mut usage_div = Element::new("span")
        .append(Element::new("strong").text("📊 Usage {"))
        .append(
            Element::new("span")
                .append(
                    Element::new("span.usage-item").text(format!("input: {}", usage.prompt_tokens)),
                )
                .append(Element::new("span.usage-item").text(format!("cached: {}", cached_display)))
                .append(
                    Element::new("span.usage-item")
                        .text(format!("output: {}", usage.completion_tokens)),
                )
                .append(
                    Element::new("span.usage-item").text(format!("total: {}", usage.total_tokens)),
                ),
        )
        .append(Element::new("strong").text("}"));

    if let Some(cost) = usage.cost {
        usage_div = usage_div
            .append(Element::new("span.usage-item.usage-cost").text(format!("Cost: ${:.4}", cost)));
    }

    usage_div
}

fn create_conversation_context_section(conversation: &Conversation) -> Element {
    let section = Element::new("div.section").append(Element::new("h2").text("Messages"));

    // Add context if available
    if let Some(context) = &conversation.context {
        let context_messages = Element::new("div.context-section").append(
            context.messages.iter().map(|message_entry| {
                match &**message_entry {
                    ContextMessage::Text(content_message) => {
                        // Convert role to lowercase for the class
                        let role_lowercase = content_message.role.to_string().to_lowercase();

                        let mut header =
                            Element::new("summary").text(format!("{}", content_message.role));

                        if let Some(model) = &content_message.model {
                            header = header
                                .append(Element::new("strong").text(" 🤖 model:"))
                                .append(Element::new("span").text(model));
                        }

                        // Add usage information
                        if let Some(usage) = &message_entry.usage {
                            header = header.append(create_message_usage_section(usage))
                        }

                        // Add reasoning indicator if reasoning details are present
                        let has_reasoning = content_message.reasoning_details.as_ref().is_some_and(|d| !d.is_empty())
                            || content_message.thought_signature.is_some();
                        if has_reasoning
                        {
                            header = header.append(
                                Element::new("span.reasoning-indicator").text(" 🧠 Reasoning"),
                            );
                        }

                        let message_elm =
                            Element::new(format!("details.message-card.message-{role_lowercase}"))
                                .append(header);

                        // Add thought signature
                        let mut message_elm = if let Some(sig) = &content_message.thought_signature {
                            message_elm.append(
                                Element::new("div.thought-signature")
                                    .append(Element::new("strong").text("💭 Thought Signature: "))
                                    .append(Element::new("pre").text(sig)),
                            )
                        } else {
                            message_elm
                        };

                        // Add reasoning details
                        message_elm = if let Some(reasoning_details) =
                            &content_message.reasoning_details
                        {
                            if !reasoning_details.is_empty() {
                                message_elm.append(Element::new("div.reasoning-section").append(
                                    reasoning_details.iter().map(|reasoning_detail| {
                                        if let Some(text) = &reasoning_detail.text {
                                            Element::new("div.reasoning-content")
                                                .append(
                                                    Element::new("strong").text("🧠 Reasoning: "),
                                                )
                                                .append(Element::new("pre").text(text))
                                        } else {
                                            Element::new("div")
                                        }
                                    }),
                                ))
                            } else {
                                message_elm
                            }
                        } else {
                            message_elm
                        };

                        // Add main content
                        let message_elm = message_elm.append(
                            Element::new("div.main-content")
                                .append(Element::new("pre").text(&content_message.content)),
                        );

                        // Add tool calls if any

                        if let Some(tool_calls) = &content_message.tool_calls {
                            if !tool_calls.is_empty() {
                                message_elm.append(Element::new("div").append(
                                    tool_calls.iter().map(|tool_call| {
                                        Element::new("div.tool-call")
                                            .append(
                                                Element::new("p").append(
                                                    Element::new("strong")
                                                        .text(tool_call.name.to_string()),
                                                ),
                                            )
                                            .append(tool_call.call_id.as_ref().map(|call_id| {
                                                Element::new("p")
                                                    .append(Element::new("strong").text("ID: "))
                                                    .text(call_id.as_str())
                                            }))
                                            .append(
                                                Element::new("p").append(
                                                    Element::new("strong").text("Arguments: "),
                                                ),
                                            )
                                            .append(
                                                Element::new("pre").text(
                                                    to_string_pretty(&tool_call.arguments)
                                                        .unwrap_or_default(),
                                                ),
                                            )
                                    }),
                                ))
                            } else {
                                message_elm
                            }
                        } else {
                            message_elm
                        }
                    }
                    ContextMessage::Tool(tool_result) => {
                        // Tool Message - apply error styling if the tool result is an error
                        let message_class = if tool_result.output.is_error {
                            "details.message-card.message-tool.tool-call-error"
                        } else {
                            "details.message-card.message-tool"
                        };

                        Element::new(message_class)
                            .append(
                                Element::new("summary")
                                    .append(Element::new("strong").text("Tool Result: "))
                                    .append(Element::span(tool_result.name.as_str())),
                            )
                            .append(Element::new("div.main-content").append(
                                tool_result.output.values.iter().filter_map(|value| {
                                    match value {
                                        crate::ToolValue::Text(text) => Some(
                                            Element::new("div")
                                                .append(Element::new("pre").text(text)),
                                        ),
                                        crate::ToolValue::Image(image) => {
                                            Some(Element::new("img").attr("src", image.url()))
                                        }
                                        crate::ToolValue::Document(document) => {
                                            let filename = document.filename().as_deref().unwrap_or("Document");
                                            Some(Element::new("div")
                                                .append(Element::new("strong").text("Document: "))
                                                .append(Element::new("span").text(format!("{} ({})", filename, document.mime_type())))
                                            )
                                        }
                                        crate::ToolValue::Empty => None,
                                        crate::ToolValue::AI { value, conversation_id } => {
                                            // Use anchor link to navigate within the same HTML
                                            let anchor_id = format!("conversation-{}", conversation_id);
                                            Some(
                                                Element::new("div.agent-conversation")
                                                    .append(
                                                        Element::new("p")
                                                            .append(Element::new("strong").text("🤖 Agent Conversation: "))
                                                            .append(
                                                                Element::new("a")
                                                                    .attr("href", format!("#{}", anchor_id))
                                                                    .attr("title", "Click to view agent conversation")
                                                                    .text(conversation_id.to_string())
                                                            )
                                                    )
                                                    .append(Element::new("pre").text(value)),
                                            )
                                        },
                                    }
                                }),
                            ))
                    }
                    ContextMessage::Image(image) => {
                        // Image message
                        Element::new("div.message-card.message-user")
                            .append(Element::new("strong").text("Image Attachment"))
                            .append(Element::new("img").attr("src", image.url()))
                    }
                    ContextMessage::Document(document) => {
                        // Document message
                        let filename = document.filename().as_deref().unwrap_or("Document");
                        Element::new("div.message-card.message-user")
                            .append(Element::new("strong").text("Document Attachment"))
                            .append(Element::new("p").text(format!("{} ({})", filename, document.mime_type())))
                    }
                }
            }),
        );

        // Create tool choice section if available
        let context_elm = if let Some(tool_choice) = &context.tool_choice {
            context_messages
                .append(Element::new("strong").text("Tool Choice"))
                .append(Element::new("div.tool-choice").append(
                    Element::new("pre").text(to_string_pretty(tool_choice).unwrap_or_default()),
                ))
        } else {
            context_messages
        };

        // Add temperature if available
        let context_elm = if let Some(temperature) = context.temperature {
            context_elm.append(
                Element::new("p")
                    .append(Element::new("strong").text("Temperature: "))
                    .text(format!("{temperature}")),
            )
        } else {
            context_elm
        };

        section.append(context_elm)
    } else {
        section.append(Element::new("p").text("No context available"))
    }
}

#[cfg(test)]
mod tests {
    use forge_test_kit::json_fixture;

    use super::*;

    #[tokio::test]
    async fn test_render_conversation_html_snapshot() {
        // Load the conversation from the fixture file
        let conversation: Conversation = json_fixture!("tests/fixtures/conversation.json").await;

        // Render the HTML
        let html = render_conversation_html(&conversation);

        // Convert HTML string to bytes for binary snapshot
        let html_bytes = html.into_bytes();

        // Binary snapshot with exact .html extension
        insta::assert_binary_snapshot!("conversation.html", html_bytes);
    }
}
