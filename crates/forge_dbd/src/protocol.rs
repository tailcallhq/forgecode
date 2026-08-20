// The conversation-write request variants are executed by the daemon server
// (bin-only `server` module) and exercised by the crate's tests; the
// forge_app client wiring is still pending. Allow dead_code until then.
#![allow(dead_code)]

use std::io;
#[cfg(windows)]
use std::path::Path;

use forge_domain::{Conversation, ConversationId};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    UpsertConversation {
        conversation: Conversation,
        /// Client-supplied workspace id (hash of the client's cwd). The
        /// daemon derives its own workspace id from ITS current directory,
        /// which diverges from the client's in `--directory` mode (path
        /// canonicalization on Windows); `None` falls back to the daemon's
        /// own derivation.
        workspace_id: Option<i64>,
    },
    UpsertConversationRef {
        conversation: Conversation,
        workspace_id: Option<i64>,
    },
    UpdateParentId {
        conversation_id: ConversationId,
        new_parent_id: Option<ConversationId>,
    },
    DeleteConversation {
        conversation_id: ConversationId,
        workspace_id: i64,
    },
    OptimizeFts,
    RefreshFts,
    CheckpointWal,
    /// Health probe: returns daemon status without side effects.
    Ping,
}

/// Status returned by a [`Request::Ping`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Seconds the daemon has been running.
    pub uptime_secs: u64,
    /// Number of write requests currently queued (not yet flushed to disk).
    pub queue_depth: usize,
    /// Whether the database file/path is reachable (existence check for now).
    pub db_reachable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Ack,
    Error {
        message: String,
    },
    /// Response to a [`Request::Ping`].
    Health(HealthStatus),
}

/// Async length-prefixed frame writer: writes u32 length prefix + serialized
/// data
pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> io::Result<()> {
    // JSON codec (the P3 design's "debugging-friendly alternate"): the domain
    // types (Conversation / Metrics / Context) rely on `skip_serializing_if`
    // extensively, which bincode — a positional format — cannot round-trip
    // (encode-then-decode of the same value fails). JSON is self-describing,
    // so serde defaults correctly fill the omitted fields.
    let serialized = serde_json::to_vec(value)
        .map_err(|e| io::Error::other(format!("serde_json error: {e}")))?;
    let len = serialized.len() as u32;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(&serialized).await?;
    Ok(())
}

/// Async length-prefixed frame reader: reads u32 length prefix + deserializes
/// data
pub async fn read_frame<R: AsyncRead + Unpin, T: for<'de> Deserialize<'de>>(
    reader: &mut R,
) -> io::Result<T> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    serde_json::from_slice(&buf).map_err(|e| io::Error::other(format!("serde_json error: {e}")))
}

/// Derives the Windows named-pipe name for a daemon socket path.
///
/// Windows has no Unix domain sockets, so the daemon listens on a named pipe
/// instead. The name is derived deterministically from the socket path so
/// client and server agree without extra configuration: every character that
/// is not alphanumeric / `.` / `-` is folded into `-`, and the result is
/// prefixed with `\\.\pipe\forge-dbd-`. Unix never uses this.
#[cfg(windows)]
pub fn named_pipe_name(socket_path: &Path) -> String {
    let raw = socket_path.to_string_lossy().to_lowercase();
    let mut sanitized = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
            sanitized.push(c);
        } else {
            sanitized.push('-');
        }
    }
    format!(r"\\.\pipe\forge-dbd-{sanitized}")
}
