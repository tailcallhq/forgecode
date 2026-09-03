//! Unit tests for the `forge_dbd::protocol` module.
//!
//! Covers Request/Response serialization roundtrips, ConversationMutation
//! variants, HealthStatus defaults, write_frame/read_frame wire roundtrip,
//! and the Windows named_pipe_name helper.

use forge_dbd::protocol::{
    ConversationMutation, HealthStatus, LEGACY_PROTOCOL_VERSION, MUTATION_PROTOCOL_VERSION,
    Request, Response,
};
use forge_domain::{Conversation, ConversationId};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
    let json = serde_json::to_string(value).expect("serialize");
    serde_json::from_str(&json).expect("deserialize")
}

fn sample_conversation() -> Conversation {
    Conversation::generate().title(Some("test-conversation".to_string()))
}

// ---------------------------------------------------------------------------
// Protocol constants
// ---------------------------------------------------------------------------

#[test]
fn mutation_protocol_version_is_two() {
    assert_eq!(MUTATION_PROTOCOL_VERSION, 2);
}

#[test]
fn legacy_protocol_version_is_one() {
    assert_eq!(LEGACY_PROTOCOL_VERSION, 1);
}

// ---------------------------------------------------------------------------
// Request serialization roundtrip — every variant
// ---------------------------------------------------------------------------

#[test]
fn request_ping_roundtrip() {
    let req = Request::Ping;
    let de = roundtrip(&req);
    assert!(matches!(de, Request::Ping));
}

#[test]
fn request_optimize_fts_roundtrip() {
    let req = Request::OptimizeFts;
    let de = roundtrip(&req);
    assert!(matches!(de, Request::OptimizeFts));
}

#[test]
fn request_refresh_fts_roundtrip() {
    let req = Request::RefreshFts;
    let de = roundtrip(&req);
    assert!(matches!(de, Request::RefreshFts));
}

#[test]
fn request_checkpoint_wal_roundtrip() {
    let req = Request::CheckpointWal;
    let de = roundtrip(&req);
    assert!(matches!(de, Request::CheckpointWal));
}

#[test]
fn request_upsert_conversation_roundtrip() {
    let conv = sample_conversation();
    let req = Request::UpsertConversation { conversation: conv };
    let de = roundtrip(&req);
    match de {
        Request::UpsertConversation { conversation } => {
            assert_eq!(conversation.title, Some("test-conversation".to_string()));
        }
        _ => panic!("expected UpsertConversation"),
    }
}

#[test]
fn request_upsert_conversation_ref_roundtrip() {
    let conv = sample_conversation();
    let req = Request::UpsertConversationRef { conversation: conv };
    let de = roundtrip(&req);
    match de {
        Request::UpsertConversationRef { conversation } => {
            assert_eq!(conversation.title, Some("test-conversation".to_string()));
        }
        _ => panic!("expected UpsertConversationRef"),
    }
}

#[test]
fn request_update_parent_id_roundtrip() {
    let cid = ConversationId::generate();
    let new_parent = Some(ConversationId::generate());
    let req = Request::UpdateParentId { conversation_id: cid, new_parent_id: new_parent };
    let de = roundtrip(&req);
    match de {
        Request::UpdateParentId { conversation_id, new_parent_id } => {
            assert_eq!(conversation_id, cid);
            assert!(new_parent_id.is_some());
        }
        _ => panic!("expected UpdateParentId"),
    }
}

#[test]
fn request_delete_conversation_roundtrip() {
    let cid = ConversationId::generate();
    let req = Request::DeleteConversation { conversation_id: cid };
    let de = roundtrip(&req);
    match de {
        Request::DeleteConversation { conversation_id } => {
            assert_eq!(conversation_id, cid);
        }
        _ => panic!("expected DeleteConversation"),
    }
}

#[test]
fn request_mutation_v2_upsert_roundtrip() {
    let conv = sample_conversation();
    let ws_id = 12345i64;
    let req = Request::MutationV2 {
        workspace_id: ws_id,
        mutation: ConversationMutation::UpsertConversation {
            conversation: conv,
            workspace_id: None,
        },
    };
    let de = roundtrip(&req);
    match de {
        Request::MutationV2 { workspace_id, mutation } => {
            assert_eq!(workspace_id, ws_id);
            match mutation {
                ConversationMutation::UpsertConversation { conversation, workspace_id: ws } => {
                    assert_eq!(conversation.title, Some("test-conversation".to_string()));
                    assert!(ws.is_none());
                }
                _ => panic!("expected UpsertConversation inside MutationV2"),
            }
        }
        _ => panic!("expected MutationV2"),
    }
}

#[test]
fn request_mutation_v2_upsert_with_workspace_roundtrip() {
    let conv = sample_conversation();
    let req = Request::MutationV2 {
        workspace_id: 99,
        mutation: ConversationMutation::UpsertConversation {
            conversation: conv,
            workspace_id: Some(42),
        },
    };
    let de = roundtrip(&req);
    match de {
        Request::MutationV2 { workspace_id, mutation } => {
            assert_eq!(workspace_id, 99);
            match mutation {
                ConversationMutation::UpsertConversation { workspace_id: ws, .. } => {
                    assert_eq!(ws, Some(42));
                }
                _ => panic!("expected UpsertConversation"),
            }
        }
        _ => panic!("expected MutationV2"),
    }
}

#[test]
fn request_mutation_v2_update_parent_id_roundtrip() {
    let cid = ConversationId::generate();
    let parent = Some(ConversationId::generate());
    let req = Request::MutationV2 {
        workspace_id: 10,
        mutation: ConversationMutation::UpdateParentId {
            conversation_id: cid,
            new_parent_id: parent,
        },
    };
    let de = roundtrip(&req);
    match de {
        Request::MutationV2 { mutation, .. } => match mutation {
            ConversationMutation::UpdateParentId { conversation_id, new_parent_id } => {
                assert_eq!(conversation_id, cid);
                assert!(new_parent_id.is_some());
            }
            _ => panic!("expected UpdateParentId"),
        },
        _ => panic!("expected MutationV2"),
    }
}

#[test]
fn request_mutation_v2_delete_roundtrip() {
    let cid = ConversationId::generate();
    let req = Request::MutationV2 {
        workspace_id: 5,
        mutation: ConversationMutation::DeleteConversation { conversation_id: cid },
    };
    let de = roundtrip(&req);
    match de {
        Request::MutationV2 { mutation, .. } => match mutation {
            ConversationMutation::DeleteConversation { conversation_id } => {
                assert_eq!(conversation_id, cid);
            }
            _ => panic!("expected DeleteConversation"),
        },
        _ => panic!("expected MutationV2"),
    }
}

#[test]
fn request_mutation_v2_upsert_ref_roundtrip() {
    let conv = sample_conversation();
    let req = Request::MutationV2 {
        workspace_id: 7,
        mutation: ConversationMutation::UpsertConversationRef {
            conversation: conv,
            workspace_id: Some(3),
        },
    };
    let de = roundtrip(&req);
    match de {
        Request::MutationV2 { mutation, .. } => match mutation {
            ConversationMutation::UpsertConversationRef { conversation, workspace_id } => {
                assert_eq!(conversation.title, Some("test-conversation".to_string()));
                assert_eq!(workspace_id, Some(3));
            }
            _ => panic!("expected UpsertConversationRef"),
        },
        _ => panic!("expected MutationV2"),
    }
}

// ---------------------------------------------------------------------------
// Response serialization roundtrip — every variant
// ---------------------------------------------------------------------------

#[test]
fn response_ack_roundtrip() {
    let resp = Response::Ack;
    let de = roundtrip(&resp);
    assert!(matches!(de, Response::Ack));
}

#[test]
fn response_error_roundtrip() {
    let msg = "something went wrong".to_string();
    let resp = Response::Error { message: msg.clone() };
    let de = roundtrip(&resp);
    match de {
        Response::Error { message } => assert_eq!(message, msg),
        _ => panic!("expected Error"),
    }
}

#[test]
fn response_health_roundtrip() {
    let status = HealthStatus {
        protocol_version: MUTATION_PROTOCOL_VERSION,
        uptime_secs: 3600,
        queue_depth: 42,
        db_reachable: true,
    };
    let resp = Response::Health(status);
    let de = roundtrip(&resp);
    match de {
        Response::Health(h) => {
            assert_eq!(h.protocol_version, MUTATION_PROTOCOL_VERSION);
            assert_eq!(h.uptime_secs, 3600);
            assert_eq!(h.queue_depth, 42);
            assert!(h.db_reachable);
        }
        _ => panic!("expected Health"),
    }
}

// ---------------------------------------------------------------------------
// HealthStatus defaults — missing protocol_version falls back to legacy
// ---------------------------------------------------------------------------

#[test]
fn health_status_deserializes_with_missing_protocol_version() {
    let json = r#"{"uptime_secs": 100, "queue_depth": 0, "db_reachable": true}"#;
    let status: HealthStatus = serde_json::from_str(json).expect("deserialize");
    assert_eq!(status.protocol_version, LEGACY_PROTOCOL_VERSION);
    assert_eq!(status.uptime_secs, 100);
    assert_eq!(status.queue_depth, 0);
    assert!(status.db_reachable);
}

#[test]
fn health_status_deserializes_with_explicit_protocol_version() {
    let json =
        r#"{"protocol_version": 2, "uptime_secs": 0, "queue_depth": 0, "db_reachable": false}"#;
    let status: HealthStatus = serde_json::from_str(json).expect("deserialize");
    assert_eq!(status.protocol_version, 2);
    assert!(!status.db_reachable);
}

// ---------------------------------------------------------------------------
// ConversationMutation — all four variants roundtrip
// ---------------------------------------------------------------------------

#[test]
fn mutation_upsert_conversation_roundtrip() {
    let conv = sample_conversation();
    let mutation =
        ConversationMutation::UpsertConversation { conversation: conv, workspace_id: Some(100) };
    let de = roundtrip(&mutation);
    match de {
        ConversationMutation::UpsertConversation { conversation, workspace_id } => {
            assert_eq!(conversation.title, Some("test-conversation".to_string()));
            assert_eq!(workspace_id, Some(100));
        }
        _ => panic!("expected UpsertConversation"),
    }
}

#[test]
fn mutation_upsert_conversation_ref_roundtrip() {
    let conv = sample_conversation();
    let mutation =
        ConversationMutation::UpsertConversationRef { conversation: conv, workspace_id: None };
    let de = roundtrip(&mutation);
    match de {
        ConversationMutation::UpsertConversationRef { conversation, workspace_id } => {
            assert_eq!(conversation.title, Some("test-conversation".to_string()));
            assert!(workspace_id.is_none());
        }
        _ => panic!("expected UpsertConversationRef"),
    }
}

#[test]
fn mutation_update_parent_id_roundtrip() {
    let cid = ConversationId::generate();
    let parent = Some(ConversationId::generate());
    let mutation =
        ConversationMutation::UpdateParentId { conversation_id: cid, new_parent_id: parent };
    let de = roundtrip(&mutation);
    match de {
        ConversationMutation::UpdateParentId { conversation_id, new_parent_id } => {
            assert_eq!(conversation_id, cid);
            assert!(new_parent_id.is_some());
        }
        _ => panic!("expected UpdateParentId"),
    }
}

#[test]
fn mutation_update_parent_id_none_roundtrip() {
    let cid = ConversationId::generate();
    let mutation =
        ConversationMutation::UpdateParentId { conversation_id: cid, new_parent_id: None };
    let de = roundtrip(&mutation);
    match de {
        ConversationMutation::UpdateParentId { new_parent_id, .. } => {
            assert!(new_parent_id.is_none());
        }
        _ => panic!("expected UpdateParentId"),
    }
}

#[test]
fn mutation_delete_conversation_roundtrip() {
    let cid = ConversationId::generate();
    let mutation = ConversationMutation::DeleteConversation { conversation_id: cid };
    let de = roundtrip(&mutation);
    match de {
        ConversationMutation::DeleteConversation { conversation_id } => {
            assert_eq!(conversation_id, cid);
        }
        _ => panic!("expected DeleteConversation"),
    }
}

// ---------------------------------------------------------------------------
// write_frame / read_frame wire roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn write_read_frame_request_roundtrip() {
    let mut buf = Vec::new();

    // Write a Ping request
    forge_dbd::protocol::write_frame(&mut buf, &Request::Ping)
        .await
        .expect("write_frame");

    // Read it back
    let de: Request = forge_dbd::protocol::read_frame(&mut buf.as_slice())
        .await
        .expect("read_frame");
    assert!(matches!(de, Request::Ping));
}

#[tokio::test]
async fn write_read_frame_response_ack_roundtrip() {
    let mut buf = Vec::new();

    forge_dbd::protocol::write_frame(&mut buf, &Response::Ack)
        .await
        .expect("write_frame");

    let de: Response = forge_dbd::protocol::read_frame(&mut buf.as_slice())
        .await
        .expect("read_frame");
    assert!(matches!(de, Response::Ack));
}

#[tokio::test]
async fn write_read_frame_response_error_roundtrip() {
    let mut buf = Vec::new();
    let resp = Response::Error { message: "test error".to_string() };

    forge_dbd::protocol::write_frame(&mut buf, &resp)
        .await
        .expect("write_frame");

    let de: Response = forge_dbd::protocol::read_frame(&mut buf.as_slice())
        .await
        .expect("read_frame");
    match de {
        Response::Error { message } => assert_eq!(message, "test error"),
        _ => panic!("expected Error"),
    }
}

#[tokio::test]
async fn write_read_frame_health_roundtrip() {
    let mut buf = Vec::new();
    let status = HealthStatus {
        protocol_version: MUTATION_PROTOCOL_VERSION,
        uptime_secs: 42,
        queue_depth: 0,
        db_reachable: true,
    };
    let resp = Response::Health(status);

    forge_dbd::protocol::write_frame(&mut buf, &resp)
        .await
        .expect("write_frame");

    let de: Response = forge_dbd::protocol::read_frame(&mut buf.as_slice())
        .await
        .expect("read_frame");
    match de {
        Response::Health(h) => {
            assert_eq!(h.protocol_version, MUTATION_PROTOCOL_VERSION);
            assert_eq!(h.uptime_secs, 42);
        }
        _ => panic!("expected Health"),
    }
}

#[tokio::test]
async fn write_read_frame_mutation_v2_roundtrip() {
    let mut buf = Vec::new();
    let conv = sample_conversation();
    let req = Request::MutationV2 {
        workspace_id: 77,
        mutation: ConversationMutation::UpsertConversation {
            conversation: conv,
            workspace_id: Some(88),
        },
    };

    forge_dbd::protocol::write_frame(&mut buf, &req)
        .await
        .expect("write_frame");

    let de: Request = forge_dbd::protocol::read_frame(&mut buf.as_slice())
        .await
        .expect("read_frame");
    match de {
        Request::MutationV2 { workspace_id, mutation } => {
            assert_eq!(workspace_id, 77);
            match mutation {
                ConversationMutation::UpsertConversation { workspace_id: ws, .. } => {
                    assert_eq!(ws, Some(88));
                }
                _ => panic!("expected UpsertConversation"),
            }
        }
        _ => panic!("expected MutationV2"),
    }
}

#[tokio::test]
async fn write_read_frame_multiple_messages_sequential() {
    let mut buf = Vec::new();

    // Write multiple frames sequentially
    forge_dbd::protocol::write_frame(&mut buf, &Request::Ping)
        .await
        .unwrap();
    forge_dbd::protocol::write_frame(&mut buf, &Response::Ack)
        .await
        .unwrap();
    forge_dbd::protocol::write_frame(&mut buf, &Request::CheckpointWal)
        .await
        .unwrap();

    let mut cursor = buf.as_slice();

    let r1: Request = forge_dbd::protocol::read_frame(&mut cursor).await.unwrap();
    assert!(matches!(r1, Request::Ping));

    let r2: Response = forge_dbd::protocol::read_frame(&mut cursor).await.unwrap();
    assert!(matches!(r2, Response::Ack));

    let r3: Request = forge_dbd::protocol::read_frame(&mut cursor).await.unwrap();
    assert!(matches!(r3, Request::CheckpointWal));
}

// ---------------------------------------------------------------------------
// Named pipe name derivation (Windows only)
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn named_pipe_name_alphanumeric_path() {
    use forge_dbd::protocol::named_pipe_name;
    use std::path::PathBuf;

    let path = PathBuf::from(r"C:\Users\test\.forge\forge-dbd.sock");
    let name = named_pipe_name(&path);
    assert!(
        name.starts_with(r"\\.\pipe\forge-dbd-"),
        "pipe name should have correct prefix: {name}"
    );
    // Alphanumeric, dots, and hyphens are preserved
    assert!(
        name.contains("forge-dbd-"),
        "should contain forge-dbd-: {name}"
    );
}

#[cfg(windows)]
#[test]
fn named_pipe_name_special_chars_folded() {
    use forge_dbd::protocol::named_pipe_name;
    use std::path::PathBuf;

    let path = PathBuf::from(r"C:\Users\test dir\path with spaces\forge.sock");
    let name = named_pipe_name(&path);
    // The pipe prefix is "\\.\pipe\forge-dbd-" which contains backslashes.
    // Verify the *sanitized suffix* (after the prefix) has no backslashes.
    let prefix = r"\\.\pipe\forge-dbd-";
    assert!(
        name.starts_with(prefix),
        "pipe name should have correct prefix: {name}"
    );
    let suffix = &name[prefix.len()..];
    assert!(
        !suffix.contains('\\'),
        "sanitized suffix should not contain backslashes: {suffix}"
    );
}

#[cfg(windows)]
#[test]
fn named_pipe_name_lowercase() {
    use forge_dbd::protocol::named_pipe_name;
    use std::path::PathBuf;

    let path = PathBuf::from(r"C:\Users\UPPERCASE\Forge.sock");
    let name = named_pipe_name(&path);
    // The implementation lowercases the path
    assert!(
        name.chars()
            .all(|c| c.is_ascii_lowercase() || !c.is_ascii_alphabetic()),
        "pipe name should be lowercase: {name}"
    );
}
