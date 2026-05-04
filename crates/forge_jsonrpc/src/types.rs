/// JSON-RPC 2.0 request parameter types and response DTOs

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Params for `conversation.create`
#[derive(Debug, Deserialize)]
pub struct CreateConversationParams {
    pub title: Option<String>,
}

/// Response for `conversation.create`
#[derive(Debug, Clone, Serialize)]
pub struct CreateConversationResponse {
    pub id: String,
    pub title: Option<String>,
    pub created_at: String,
}

/// Params for `chat.stream`
#[derive(Debug, Deserialize)]
pub struct ChatStreamParams {
    pub conversation_id: String,
    pub message: String,
    #[serde(default)]
    pub include_reasoning: Option<bool>,
}

/// A notification emitted as a subscription message during `chat.stream`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum StreamNotification {
    #[serde(rename = "message")]
    Message {
        content: String,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        content: String,
    },
    #[serde(rename = "complete")]
    Complete,
    #[serde(rename = "error")]
    Error {
        message: String,
    },
}

/// A message sent through the subscription channel.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum StreamMessage {
    Chunk {
        data: StreamNotification,
    },
    Complete {
        #[serde(rename = "type")]
        typ: String,
    },
    Error {
        message: String,
    },
}

/// Response for `rpc.methods` / `get_methods`
#[derive(Debug, Clone, Serialize)]
pub struct MethodInfo {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_stream_message_complete_serialization() {
        let msg = StreamMessage::Complete { typ: "complete".into() };
        let actual = serde_json::to_value(&msg).unwrap();
        let expected = json!({"type": "complete"});
        assert_eq!(actual, expected);
        // Must NOT be null
        assert!(!actual.is_null());
    }

    #[test]
    fn test_stream_message_chunk_message() {
        let msg = StreamMessage::Chunk {
            data: StreamNotification::Message {
                content: "hello".into(),
            },
        };
        let actual = serde_json::to_value(&msg).unwrap();
        let expected = json!({"data": {"type": "message", "content": "hello"}});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_stream_message_chunk_reasoning() {
        let msg = StreamMessage::Chunk {
            data: StreamNotification::Reasoning {
                content: "thinking...".into(),
            },
        };
        let actual = serde_json::to_value(&msg).unwrap();
        let expected = json!({"data": {"type": "reasoning", "content": "thinking..."}});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_stream_message_error() {
        let msg = StreamMessage::Error {
            message: "something went wrong".into(),
        };
        let actual = serde_json::to_value(&msg).unwrap();
        let expected = json!({"message": "something went wrong"});
        assert_eq!(actual, expected);
    }

    /// Simulate the full JSON-RPC notification envelope that wraps a
    /// StreamMessage to verify the completion result is explicit.
    #[test]
    fn test_completion_notification_has_explicit_type() {
        let complete = StreamMessage::Complete { typ: "complete".into() };
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "chat.notification",
            "params": {
                "subscription": "sub-1",
                "result": serde_json::to_value(&complete).unwrap()
            }
        });

        let params = &notification["params"];
        assert!(params["result"].is_object(), "result should be an object, not null");
        assert_eq!(params["result"]["type"], "complete");
    }
}
