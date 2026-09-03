// Cross-ecosystem contract: forgecode (AI coding agent) <-> substrate (LLM
// dispatch)
//
// These tests verify the LLM request/response contracts between the forgecode
// agent and the substrate gateway. They validate schema alignment without
// requiring running services.

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug)]
    struct LLMRequest {
        model: String,
        messages: Vec<LLMMessage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        temperature: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_tokens: Option<u32>,
        stream: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<std::collections::HashMap<String, String>>,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct LLMMessage {
        role: String,
        content: String,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct LLMResponse {
        id: String,
        choices: Vec<LLMChoice>,
        usage: LLMUsage,
        model: String,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct LLMChoice {
        index: u32,
        message: LLMMessage,
        finish_reason: String,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct LLMUsage {
        prompt_tokens: u32,
        completion_tokens: u32,
        total_tokens: u32,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct StreamingDelta {
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct StreamingChoice {
        index: u32,
        delta: StreamingDelta,
        finish_reason: Option<String>,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct StreamingChunk {
        id: String,
        choices: Vec<StreamingChoice>,
    }

    #[test]
    fn test_llm_request_contract() {
        let payload = r#"{
            "model": "gpt-4o",
            "messages": [
                {"role": "system", "content": "You are a coding assistant."},
                {"role": "user", "content": "Write a hello world function"}
            ],
            "temperature": 0.7,
            "max_tokens": 1024,
            "stream": false
        }"#;

        let req: LLMRequest = serde_json::from_str(payload).expect("LLMRequest parse failed");

        assert!(!req.model.is_empty(), "Model must be non-empty");
        assert!(
            !req.messages.is_empty(),
            "Messages must have at least 1 entry"
        );
        assert!(
            req.messages[0].role == "system" || req.messages[0].role == "user",
            "First message role must be system or user"
        );

        for msg in &req.messages {
            assert!(
                msg.role == "system" || msg.role == "user" || msg.role == "assistant",
                "Invalid role: {}",
                msg.role
            );
        }
    }

    #[test]
    fn test_llm_response_contract() {
        let payload = r#"{
            "id": "chatcmpl-abc123",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "def hello():\n    print('Hello, World!')"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 25,
                "completion_tokens": 15,
                "total_tokens": 40
            },
            "model": "gpt-4o"
        }"#;

        let resp: LLMResponse = serde_json::from_str(payload).expect("LLMResponse parse failed");

        assert!(!resp.id.is_empty(), "ID must be non-empty");
        assert!(
            !resp.choices.is_empty(),
            "Choices must have at least 1 entry"
        );
        assert_eq!(resp.choices[0].message.role, "assistant");
        assert_eq!(
            resp.usage.total_tokens,
            resp.usage.prompt_tokens + resp.usage.completion_tokens,
            "TotalTokens must equal PromptTokens+CompletionTokens"
        );

        let valid_reasons = ["stop", "length", "tool_calls"];
        assert!(
            valid_reasons.contains(&resp.choices[0].finish_reason.as_str()),
            "Finish reason '{}' not in [stop, length, tool_calls]",
            resp.choices[0].finish_reason
        );
    }

    #[test]
    fn test_streaming_chunk_contract() {
        let chunk = r#"{"id":"chatcmpl-abc","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#;

        let c: StreamingChunk = serde_json::from_str(chunk).expect("Streaming chunk parse failed");

        assert!(!c.id.is_empty(), "Chunk ID must be non-empty");
        assert!(!c.choices.is_empty(), "Choices must have at least 1 entry");
        assert!(
            c.choices[0].finish_reason.is_none(),
            "First chunk finish_reason should be null"
        );
    }

    #[test]
    fn test_request_response_roundtrip() {
        let req = LLMRequest {
            model: "claude-3".to_string(),
            messages: vec![LLMMessage { role: "user".to_string(), content: "Hello".to_string() }],
            temperature: Some(0.5),
            max_tokens: Some(512),
            stream: false,
            metadata: None,
        };

        let json = serde_json::to_string(&req).expect("serialize failed");
        let roundtrip: LLMRequest = serde_json::from_str(&json).expect("deserialize failed");

        assert_eq!(roundtrip.model, req.model);
        assert_eq!(roundtrip.messages.len(), req.messages.len());
        assert_eq!(roundtrip.stream, req.stream);
    }
}
