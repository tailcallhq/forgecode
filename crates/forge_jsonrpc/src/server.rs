use std::sync::Arc;

use forge_api::API;
use forge_domain::{ChatRequest, ConversationId, Event, EventValue};
use futures::StreamExt;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::{RpcModule, SubscriptionMessage};
use serde_json::{json, Value};
use tracing::debug;

use crate::error::{map_error, not_found, ErrorCode};
use crate::types::*;

/// Helper to serialize a value, mapping serialization failures to JSON-RPC
/// errors.
fn to_json_response<T: serde::Serialize>(value: T) -> Result<Value, ErrorObjectOwned> {
    serde_json::to_value(value).map_err(|e| {
        ErrorObjectOwned::owned(
            ErrorCode::INTERNAL_ERROR,
            format!("Failed to serialize response: {e}"),
            None::<()>,
        )
    })
}

/// STDIO-based JSON-RPC server wrapping the Forge API.
///
/// Registers all JSON-RPC methods and subscriptions on an `RpcModule`.
/// The module is then driven by a [`StdioTransport`] that reads requests
/// from stdin and writes responses/subscription notifications to stdout.
pub struct JsonRpcServer<A: API> {
    api: Arc<A>,
    module: RpcModule<()>,
}

impl<A: API + 'static> JsonRpcServer<A> {
    /// Create a new JSON-RPC server, register all methods, and return the
    /// ready-to-use instance.
    pub fn new(api: Arc<A>) -> Self {
        let mut server = Self { api, module: RpcModule::new(()) };
        server.register_methods();
        server
    }

    /// Consume the server and return the underlying RpcModule.
    pub fn into_module(self) -> RpcModule<()> {
        self.module
    }

    // ------------------------------------------------------------------
    // Registration
    // ------------------------------------------------------------------

    fn register_methods(&mut self) {
        self.register_discovery();
        self.register_conversation();
        self.register_chat();
    }

    // ------------------------------------------------------------------
    // Discovery
    // ------------------------------------------------------------------

    fn register_discovery(&mut self) {
        // Build methods list once
        let methods_list: Vec<MethodInfo> = vec![
            MethodInfo {
                name: "rpc.methods".into(),
                description: "List all available JSON-RPC methods.".into(),
                params: None,
                result: None,
            },
            MethodInfo {
                name: "rpc.discover".into(),
                description: "List all available JSON-RPC methods (alias for rpc.methods).".into(),
                params: None,
                result: None,
            },
            MethodInfo {
                name: "get_methods".into(),
                description: "List all available JSON-RPC methods (alias for rpc.methods).".into(),
                params: None,
                result: None,
            },
            MethodInfo {
                name: "conversation.create".into(),
                description: "Create a new conversation with an optional title.".into(),
                params: Some(json!({"title": "string (optional)"})),
                result: Some(json!({"id": "uuid", "title": "string", "created_at": "rfc3339"})),
            },
            MethodInfo {
                name: "chat.stream".into(),
                description: "Stream chat responses for a conversation. Subscription.".into(),
                params: Some(json!({
                    "conversation_id": "uuid (required)",
                    "message": "string (required)",
                    "include_reasoning": "boolean (optional)"
                })),
                result: None,
            },
        ];

        // rpc.methods
        let methods_for_rpc = methods_list.clone();
        self.module
            .register_async_method("rpc.methods", move |_, _, _| {
                let methods = methods_for_rpc.clone();
                async move { to_json_response(methods) }
            })
            .expect("Failed to register rpc.methods");

        // rpc.discover — alias
        let methods_for_discover = methods_list.clone();
        self.module
            .register_async_method("rpc.discover", move |_, _, _| {
                let methods = methods_for_discover.clone();
                async move { to_json_response(methods) }
            })
            .expect("Failed to register rpc.discover");

        // get_methods — alias
        let methods_for_get = methods_list.clone();
        self.module
            .register_async_method("get_methods", move |_, _, _| {
                let methods = methods_for_get.clone();
                async move { to_json_response(methods) }
            })
            .expect("Failed to register get_methods");
    }

    // ------------------------------------------------------------------
    // Conversation
    // ------------------------------------------------------------------

    fn register_conversation(&mut self) {
        let api = self.api.clone();
        self.module
            .register_async_method("conversation.create", move |params, _, _| {
                let api = api.clone();
                async move {
                    let p: CreateConversationParams = params.parse()?;
                    let conversation = api.create_conversation(p.title).await.map_err(map_error)?;
                    let response = CreateConversationResponse {
                        id: conversation.id.into_string(),
                        title: conversation.title,
                        created_at: conversation.metadata.created_at.to_rfc3339(),
                    };
                    to_json_response(response)
                }
            })
            .expect("Failed to register conversation.create");
    }

    // ------------------------------------------------------------------
    // Chat
    // ------------------------------------------------------------------

    fn register_chat(&mut self) {
        let api = self.api.clone();

        self.module
            .register_subscription(
                "chat.stream",
                "chat.notification",
                "chat.stream.unsubscribe",
                move |params, pending, _, _| {
                    let api = api.clone();
                    async move {
                        // Parse params.  Use pending.reject() instead of `?`
                        // because jsonrpsee's subscription infrastructure
                        // hangs when the future returns Err without calling
                        // accept/reject on the pending sink.
                        let p: ChatStreamParams = match params.parse() {
                            Ok(p) => p,
                            Err(e) => {
                                pending
                                    .reject(ErrorObjectOwned::owned(
                                        ErrorCode::INVALID_PARAMS,
                                        format!("Invalid params: {e}"),
                                        None::<()>,
                                    ))
                                    .await;
                                return Ok(());
                            }
                        };

                        // Parse and validate conversation_id early.
                        let conversation_id = match ConversationId::parse(&p.conversation_id) {
                            Ok(id) => id,
                            Err(e) => {
                                pending
                                    .reject(ErrorObjectOwned::owned(
                                        ErrorCode::INVALID_PARAMS,
                                        format!("Invalid conversation_id: {e}"),
                                        None::<()>,
                                    ))
                                    .await;
                                return Ok(());
                            }
                        };

                        // Validate conversation exists before starting the
                        // stream.
                        match api.conversation(&conversation_id).await {
                            Ok(Some(_)) => { /* OK */ }
                            Ok(None) => {
                                pending
                                    .reject(not_found("Conversation", &p.conversation_id))
                                    .await;
                                return Ok(());
                            }
                            Err(e) => {
                                pending
                                    .reject(map_error(e))
                                    .await;
                                return Ok(());
                            }
                        }

                        let include_reasoning = p.include_reasoning.unwrap_or(false);

                        // Accept the subscription
                        let sink = match pending.accept().await {
                            Ok(sink) => sink,
                            Err(_) => return Ok(()),
                        };

                        let event = Event::new(EventValue::text(p.message));
                        let chat_request = ChatRequest::new(event, conversation_id);

                        // Start the chat stream
                        let stream = match api.chat(chat_request).await {
                            Ok(stream) => stream,
                            Err(e) => {
                                let err_msg = StreamMessage::Error {
                                    message: format!("{e:#}"),
                                };
                                let sub_msg =
                                    SubscriptionMessage::from_json(&err_msg).unwrap_or_else(|_| {
                                        SubscriptionMessage::from_json(
                                            &json!({"status": "error"}),
                                        )
                                        .expect("fallback should never fail")
                                    });
                                let _ = sink.send(sub_msg).await;
                                return Ok(());
                            }
                        };

                        tokio::pin!(stream);
                        loop {
                            let item = stream.next().await;
                            let msg = match item {
                                Some(Ok(resp)) => {
                                    match resp {
                                        forge_domain::ChatResponse::TaskMessage { content } => {
                                            let is_tool_input = matches!(&content, forge_domain::ChatResponseContent::ToolInput(_));
                                            if is_tool_input {
                                                // Skip tool input notifications for JSON-RPC
                                                continue;
                                            }
                                            let text = content.as_str().to_string();
                                            StreamMessage::Chunk {
                                                data: StreamNotification::Message { content: text },
                                            }
                                        }
                                        forge_domain::ChatResponse::TaskReasoning { content } => {
                                            if !include_reasoning {
                                                continue;
                                            }
                                            StreamMessage::Chunk {
                                                data: StreamNotification::Reasoning { content },
                                            }
                                        }
                                        forge_domain::ChatResponse::TaskComplete => {
                                            StreamMessage::Complete { typ: "complete".into() }
                                        }
                                        forge_domain::ChatResponse::ToolCallStart { .. } => {
                                            // Skip tool call start events for
                                            // JSON-RPC
                                            continue;
                                        }
                                        forge_domain::ChatResponse::ToolCallEnd(_) => {
                                            // Skip tool call end events for
                                            // JSON-RPC
                                            continue;
                                        }
                                        forge_domain::ChatResponse::RetryAttempt { cause, .. } => {
                                            StreamMessage::Chunk {
                                                data: StreamNotification::Error {
                                                    message: cause.into_string(),
                                                },
                                            }
                                        }
                                        forge_domain::ChatResponse::Interrupt { reason } => {
                                            StreamMessage::Chunk {
                                                data: StreamNotification::Error {
                                                    message: format!("{reason:?}"),
                                                },
                                            }
                                        }
                                    }
                                }
                                Some(Err(e)) => StreamMessage::Error {
                                    message: format!("{e:#}"),
                                },
                                None => break,
                            };

                            let sub_msg = SubscriptionMessage::from_json(&msg).unwrap_or_else(|_| {
                                SubscriptionMessage::from_json(&json!({"status": "error"}))
                                    .expect("fallback should never fail")
                            });

                            if sink.send(sub_msg).await.is_err() {
                                debug!("Client disconnected from chat stream");
                                break;
                            }
                        }

                        Ok(())
                    }
                },
            )
            .expect("Failed to register chat.stream");
    }
}
