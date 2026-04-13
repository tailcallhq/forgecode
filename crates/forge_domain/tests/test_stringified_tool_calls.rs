//! Integration test for stringified tool call arguments fix
//!
//! This test verifies that when the API sends tool call arguments as a string
//! containing JSON (like kimi-k2p5-turbo does), we properly parse it and
//! serialize it back as a JSON object when sending to the API.

use forge_domain::{Context, ContextMessage, Role};

/// Test that stringified tool call arguments from API are properly handled
///
/// This simulates the exact scenario: API sends arguments as a string
/// containing JSON, and when we serialize the conversation back to send to API,
/// it should be a proper object.
#[test]
fn test_stringified_tool_call_arguments_roundtrip() {
    // Simulate what kimi-k2p5-turbo sends: arguments as a string containing JSON
    // Note: This is what the API sends us - a JSON string value containing JSON
    // object
    let conversation_json = r#"{
        "messages": [
            {
                "text": {
                    "role": "System",
                    "content": "You are Forge."
                }
            },
            {
                "text": {
                    "role": "User",
                    "content": "Read a file"
                }
            },
            {
                "text": {
                    "role": "Assistant",
                    "content": "I'll read the file.",
                    "tool_calls": [
                        {
                            "name": "read",
                            "call_id": "call_001",
                            "arguments": "{\"file_path\": \"/test/path\", \"start_line\": 1, \"end_line\": 10}"
                        }
                    ]
                }
            }
        ]
    }"#;

    // Deserialize the conversation (this is what happens when we receive from API)
    let context: Context =
        serde_json::from_str(conversation_json).expect("Failed to parse conversation");

    // Find the assistant message with tool calls
    let assistant_msg = context
        .messages
        .iter()
        .find_map(|entry| match &entry.message {
            ContextMessage::Text(text) if text.role == Role::Assistant => Some(text),
            _ => None,
        })
        .expect("Should have assistant message");

    let tool_calls = assistant_msg
        .tool_calls
        .as_ref()
        .expect("Should have tool calls");
    assert_eq!(tool_calls.len(), 1);

    let tool_call = &tool_calls[0];
    assert_eq!(tool_call.name.as_str(), "read");

    // Verify arguments are parsed correctly (can access fields)
    let parsed_args = tool_call.arguments.parse().expect("Should parse arguments");
    assert_eq!(parsed_args["file_path"], "/test/path");
    assert_eq!(parsed_args["start_line"], 1);

    // Now serialize the context back to JSON (as we would send to API)
    let serialized = serde_json::to_string(&context).expect("Should serialize");
    println!("Serialized: {}", serialized);

    // Parse the JSON to verify structure
    let reparsed: serde_json::Value = serde_json::from_str(&serialized).expect("Should re-parse");

    // Find the assistant message
    let messages = reparsed["messages"]
        .as_array()
        .expect("Should have messages");
    let assistant_json = messages
        .iter()
        .find(|m| m["text"]["role"] == "Assistant")
        .expect("Should find assistant message");

    let tool_calls_json = assistant_json["text"]["tool_calls"]
        .as_array()
        .expect("Should have tool_calls");

    let args = &tool_calls_json[0]["arguments"];

    // THE KEY TEST: arguments should be a JSON object, not a string
    // This is what Fireworks API expects - if it's a string, we get 400 error
    assert!(
        args.is_object(),
        "CRITICAL: arguments must be a JSON object for API, not a string. Got: {}",
        args
    );

    // Verify the values are preserved correctly
    assert_eq!(args["file_path"], "/test/path");
    assert_eq!(args["start_line"], 1);
    assert_eq!(args["end_line"], 10);

    println!("SUCCESS: Stringified arguments properly converted to JSON object");
}

/// Test with patch tool (the actual error case from kimi-k2p5-turbo)
#[test]
fn test_kimi_k2p5_turbo_patch_tool_scenario() {
    // This simulates the exact error case: patch tool with stringified arguments
    let conversation_json = r#"{
        "messages": [
            {
                "text": {
                    "role": "System",
                    "content": "You are Forge."
                }
            },
            {
                "text": {
                    "role": "User",
                    "content": "Edit the file"
                }
            },
            {
                "text": {
                    "role": "Assistant",
                    "content": "I'll edit the file.",
                    "tool_calls": [
                        {
                            "name": "patch",
                            "call_id": "call_patch_001",
                            "arguments": "{\"file_path\": \"/test/file.rs\", \"old_string\": \"fn main() {\\n}\", \"new_string\": \"fn main() {\\n    println!(\\\"Hello\\\");\\n}\"}"
                        }
                    ]
                }
            }
        ]
    }"#;

    let context: Context =
        serde_json::from_str(conversation_json).expect("Failed to parse conversation");

    // Serialize back to what we'd send to API
    let serialized = serde_json::to_string(&context).expect("Should serialize");
    let reparsed: serde_json::Value = serde_json::from_str(&serialized).expect("Should parse");

    let messages = reparsed["messages"]
        .as_array()
        .expect("Should have messages");
    let assistant = messages
        .iter()
        .find(|m| m["text"]["role"] == "Assistant")
        .expect("Should find assistant");

    let tool_calls = assistant["text"]["tool_calls"]
        .as_array()
        .expect("Should have tool_calls");
    let args = &tool_calls[0]["arguments"];

    // THE CRITICAL ASSERTION for the kimi-k2p5-turbo fix
    assert!(
        args.is_object(),
        "Tool call arguments must be a JSON object when sent to API, not a string. Got: {}",
        args
    );

    // Verify nested strings with special characters are preserved
    assert_eq!(args["file_path"], "/test/file.rs");
    assert_eq!(args["old_string"], "fn main() {\n}");
    assert_eq!(
        args["new_string"],
        "fn main() {\n    println!(\"Hello\");\n}"
    );

    println!("SUCCESS: Patch tool with stringified args properly converted");
}

/// Test multiple tool calls with stringified arguments
#[test]
fn test_multiple_stringified_tool_calls() {
    let conversation_json = r#"{
        "messages": [
            {
                "text": {
                    "role": "System",
                    "content": "You are Forge."
                }
            },
            {
                "text": {
                    "role": "User",
                    "content": "Do multiple things"
                }
            },
            {
                "text": {
                    "role": "Assistant",
                    "content": "I'll do multiple things.",
                    "tool_calls": [
                        {
                            "name": "read",
                            "call_id": "call_1",
                            "arguments": "{\"file_path\": \"/file1.txt\"}"
                        },
                        {
                            "name": "patch",
                            "call_id": "call_2",
                            "arguments": "{\"file_path\": \"/file2.txt\", \"old_string\": \"a\", \"new_string\": \"b\"}"
                        }
                    ]
                }
            }
        ]
    }"#;

    let context: Context =
        serde_json::from_str(conversation_json).expect("Failed to parse conversation");
    let serialized = serde_json::to_string(&context).expect("Should serialize");
    let reparsed: serde_json::Value = serde_json::from_str(&serialized).expect("Should parse");

    let messages = reparsed["messages"]
        .as_array()
        .expect("Should have messages");
    let assistant = messages
        .iter()
        .find(|m| m["text"]["role"] == "Assistant")
        .expect("Should find assistant");

    let tool_calls = assistant["text"]["tool_calls"]
        .as_array()
        .expect("Should have tool_calls");

    assert_eq!(tool_calls.len(), 2);

    // Both tool calls should have arguments as objects, not strings
    for (i, tc) in tool_calls.iter().enumerate() {
        let args = &tc["arguments"];
        assert!(
            args.is_object(),
            "Tool call {} arguments should be a JSON object, got: {}",
            i,
            args
        );
    }

    println!("SUCCESS: Multiple stringified tool calls properly converted to JSON objects");
}

/// Test that regular JSON objects (not stringified) still work correctly
#[test]
fn test_regular_json_objects_unchanged() {
    // Normal case: arguments are already proper JSON objects
    let conversation_json = r#"{
        "messages": [
            {
                "text": {
                    "role": "System",
                    "content": "You are Forge."
                }
            },
            {
                "text": {
                    "role": "Assistant",
                    "content": "I'll read the file.",
                    "tool_calls": [
                        {
                            "name": "read",
                            "call_id": "call_001",
                            "arguments": {"file_path": "/test/path", "start_line": 1}
                        }
                    ]
                }
            }
        ]
    }"#;

    let context: Context =
        serde_json::from_str(conversation_json).expect("Failed to parse conversation");
    let serialized = serde_json::to_string(&context).expect("Should serialize");
    let reparsed: serde_json::Value = serde_json::from_str(&serialized).expect("Should parse");

    let messages = reparsed["messages"]
        .as_array()
        .expect("Should have messages");
    let assistant = messages
        .iter()
        .find(|m| m["text"]["role"] == "Assistant")
        .expect("Should find assistant");

    let tool_calls = assistant["text"]["tool_calls"]
        .as_array()
        .expect("Should have tool_calls");
    let args = &tool_calls[0]["arguments"];

    // Should still be an object
    assert!(
        args.is_object(),
        "Regular JSON objects should remain as objects"
    );
    assert_eq!(args["file_path"], "/test/path");
    assert_eq!(args["start_line"], 1);

    println!("SUCCESS: Regular JSON objects work correctly");
}
