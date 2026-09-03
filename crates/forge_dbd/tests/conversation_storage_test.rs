//! Unit tests for `conversation_storage::persist_context`.

use forge_dbd::conversation_storage::{PersistedContext, persist_context};
use forge_domain::{Context, ContextMessage, Role, TextMessage};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal `Context` with a single user message.
fn ctx_with_message(content: &str) -> Context {
    let msg = ContextMessage::Text(TextMessage::new(Role::User, content));
    Context::default().add_message(msg)
}

/// Build a `Context` that has an `initiator` but zero messages.
fn ctx_initiator_only(initiator: &str) -> Context {
    Context { initiator: Some(initiator.to_string()), ..Context::default() }
}

/// Build a `Context` with many messages so the compressed blob is non-trivial.
fn ctx_many_messages(n: usize) -> Context {
    let mut ctx = Context::default();
    for i in 0..n {
        let msg = ContextMessage::Text(TextMessage::new(
            Role::User,
            format!("message-{i}: padding content for compression"),
        ));
        ctx = ctx.add_message(msg);
    }
    ctx
}

// ---------------------------------------------------------------------------
// persist_context(None)
// ---------------------------------------------------------------------------

#[test]
fn persist_none_returns_empty_columns() {
    let result = persist_context(None);

    assert_eq!(
        result,
        PersistedContext {
            context: None,
            message_count: None,
            context_zstd: None,
            is_compressed: 0,
        }
    );
}

// ---------------------------------------------------------------------------
// persist_context with Some context containing messages
// ---------------------------------------------------------------------------

#[test]
fn persist_context_with_single_message_compresses_successfully() {
    let ctx = ctx_with_message("Hello, world!");
    let result = persist_context(Some(&ctx));

    // message_count must reflect the single message
    assert_eq!(result.message_count, Some(1));
    // zstd compression should succeed for non-trivial JSON
    assert!(
        result.context_zstd.is_some(),
        "expected compressed blob for a context with a message"
    );
    // When compression succeeds, the plaintext field is None
    assert!(result.context.is_none());
    assert_eq!(result.is_compressed, 1);
}

#[test]
fn persist_context_with_multiple_messages_records_count() {
    let ctx = ctx_many_messages(10);
    let result = persist_context(Some(&ctx));

    assert_eq!(result.message_count, Some(10));
    assert!(result.context_zstd.is_some());
    assert_eq!(result.is_compressed, 1);
}

#[test]
fn persist_context_compressed_blob_is_valid_zstd() {
    let ctx = ctx_with_message("Verify decompression roundtrip");
    let result = persist_context(Some(&ctx));

    let compressed = result.context_zstd.expect("compressed blob");
    // Decompress and verify we get valid JSON back
    let decompressed = zstd::decode_all(compressed.as_slice()).expect("zstd decompress");
    let json_str = String::from_utf8(decompressed).expect("valid utf-8");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("valid JSON");

    // The JSON should contain the messages array with our content
    let messages = parsed.get("messages").expect("messages field");
    assert!(messages.is_array());
    assert_eq!(messages.as_array().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// Empty messages — context is filtered out when messages are empty
// ---------------------------------------------------------------------------

#[test]
fn persist_context_empty_messages_no_initiator_returns_empty() {
    // Context with no messages and no initiator should be filtered to None
    let ctx = Context::default();
    let result = persist_context(Some(&ctx));

    assert_eq!(
        result,
        PersistedContext {
            context: None,
            message_count: None,
            context_zstd: None,
            is_compressed: 0,
        }
    );
}

#[test]
fn persist_context_empty_messages_with_initiator_persists() {
    let ctx = ctx_initiator_only("user");
    let result = persist_context(Some(&ctx));

    // The initiator field makes this context non-empty, so it should persist
    assert!(
        result.context_zstd.is_some() || result.context.is_some(),
        "expected persisted data when initiator is set"
    );
    assert_eq!(result.message_count, Some(0));
}

// ---------------------------------------------------------------------------
// Tool call messages roundtrip
// ---------------------------------------------------------------------------

#[test]
fn persist_context_with_tool_result_message() {
    use forge_domain::{ToolName, ToolOutput, ToolResult};

    let tool_msg = ContextMessage::Tool(ToolResult {
        name: ToolName::new("fs_search"),
        call_id: None,
        output: ToolOutput::text("search results here"),
    });
    let ctx = Context::default().add_message(tool_msg);
    let result = persist_context(Some(&ctx));

    assert_eq!(result.message_count, Some(1));
    assert!(result.context_zstd.is_some());
    assert_eq!(result.is_compressed, 1);
}

// ---------------------------------------------------------------------------
// Large context compression
// ---------------------------------------------------------------------------

#[test]
fn persist_context_large_context_compresses_to_smaller_blob() {
    let ctx = ctx_many_messages(200);
    let result = persist_context(Some(&ctx));

    let compressed = result.context_zstd.expect("compressed blob");

    // Build the uncompressed JSON for comparison
    let ctx_record_json = serde_json::to_string(&ctx).expect("serialize context");
    let uncompressed_len = ctx_record_json.len();

    // Compressed blob should be smaller (or at least not wildly larger)
    // for repetitive content like repeated "message-N: padding content"
    // We'll just verify the compressed blob exists and has a reasonable size.
    assert!(
        compressed.len() < uncompressed_len,
        "compressed ({}) should be smaller than uncompressed ({})",
        compressed.len(),
        uncompressed_len
    );
}

// ---------------------------------------------------------------------------
// Context with both messages and initiator
// ---------------------------------------------------------------------------

#[test]
fn persist_context_with_messages_and_initiator() {
    let mut ctx = ctx_with_message("user question");
    ctx.initiator = Some("user".to_string());
    let result = persist_context(Some(&ctx));

    assert_eq!(result.message_count, Some(1));
    assert!(result.context_zstd.is_some());
    assert_eq!(result.is_compressed, 1);
}

// ---------------------------------------------------------------------------
// Context with initiator=None (explicit)
// ---------------------------------------------------------------------------

#[test]
fn persist_context_with_none_initiator_and_messages() {
    let mut ctx = ctx_with_message("Hello");
    ctx.initiator = None;
    let result = persist_context(Some(&ctx));

    assert_eq!(result.message_count, Some(1));
    assert!(result.context_zstd.is_some());
    assert_eq!(result.is_compressed, 1);
}
