//! Wire protocol: Unix-domain-socket framing + JSON-RPC 2.0.
//!
//! ## Frame format
//!
//! Each message on the UDS is encoded as:
//!
//! ```text
//! +--------+--------+--------+--------+--------+--------+-----+
//! |     length (u32 big-endian)     |       utf-8 json       |
//! +--------+--------+--------+--------+--------+--------+-----+
//! \____ 4 bytes ____/              \______ length bytes ___/
//! ```
//!
//! The length header is the byte length of the UTF-8 JSON payload
//! (not including the header itself). Maximum frame size is 16 MiB,
//! which is well above the largest realistic JSON-RPC payload.
//!
//! ## JSON-RPC 2.0 + `notify` extension
//!
//! Requests follow [JSON-RPC 2.0](https://www.jsonrpc.org/specification).
//! The single extension is the `notify` method: a top-level
//! `{ "method": "notify", "params": { "topic": "...", "payload": {} } }`
//! envelope used by the server to push events (e.g. `drift.alert`) to
//! subscribed clients without an outstanding request id.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::net::UnixStream;

/// Hard cap on a single frame. 16 MiB matches the JSON-RPC spec's
/// "reasonable" limit and stops a malicious client from forcing us
/// to allocate gigabytes.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// Method name reserved for server-pushed notifications.
pub const NOTIFY_METHOD: &str = "notify";

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Method to invoke, e.g. `"agent.register"`.
    pub method: String,
    /// Structured parameters. May be a JSON object or array; serde_json
    /// keeps the original shape as `Value`.
    #[serde(default)]
    pub params: serde_json::Value,
    /// Caller-supplied correlation id. Echoed back in the response.
    pub id: serde_json::Value,
}

/// JSON-RPC 2.0 success response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcSuccess {
    pub jsonrpc: String,
    /// Result payload. Anything JSON-serialisable.
    pub result: serde_json::Value,
    /// Echoed `id` from the matching request.
    pub id: serde_json::Value,
}

/// JSON-RPC 2.0 error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError_ {
    pub jsonrpc: String,
    /// Structured error object.
    pub error: RpcErrorBody,
    /// Echoed `id`. `Null` if the id could not be determined (parse
    /// error on the request side).
    pub id: serde_json::Value,
}

/// Body of an [`RpcError_`] response. Mirrors the JSON-RPC 2.0 spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcErrorBody {
    /// Numeric error code.
    pub code: i32,
    /// Short human-readable summary.
    pub message: String,
    /// Optional structured detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Tagged union covering every wire message we emit. Lets the client
/// side decode with a single match.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcMessage {
    /// Successful reply.
    Success(RpcSuccess),
    /// Error reply.
    Error(RpcError_),
    /// Server-pushed notification (`method == "notify"`).
    Notify(NotifyEnvelope),
}

/// Server-pushed notification. The `topic` discriminates the payload
/// schema (e.g. `"drift.alert"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyEnvelope {
    pub jsonrpc: String,
    pub method: String,
    pub params: NotifyParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyParams {
    pub topic: String,
    pub payload: serde_json::Value,
}

/// JSON-RPC error codes used by the daemon.
///
/// Codes in `[-32700, -32099]` are reserved by the JSON-RPC spec. The
/// daemon reserves `-32001` and `-32002` for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcError {
    /// JSON could not be parsed. (-32700)
    ParseError,
    /// JSON payload was not a valid request. (-32600)
    InvalidRequest,
    /// Method does not exist. (-32601)
    MethodNotFound,
    /// Method parameters were invalid. (-32602)
    InvalidParams,
    /// Internal JSON-RPC error. (-32603)
    InternalError,
    /// Daemon is shutting down / not accepting requests. (-32001)
    DaemonUnavail,
    /// Requested drift tier is disabled by configuration. (-32002)
    DriftTierDisabled,
}

impl RpcError {
    /// Numeric code, as defined in the JSON-RPC spec.
    pub const fn code(self) -> i32 {
        match self {
            Self::ParseError => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::InternalError => -32603,
            Self::DaemonUnavail => -32001,
            Self::DriftTierDisabled => -32002,
        }
    }

    /// Short, human-readable description.
    pub const fn message(self) -> &'static str {
        match self {
            Self::ParseError => "Parse error",
            Self::InvalidRequest => "Invalid request",
            Self::MethodNotFound => "Method not found",
            Self::InvalidParams => "Invalid params",
            Self::InternalError => "Internal error",
            Self::DaemonUnavail => "Daemon unavailable",
            Self::DriftTierDisabled => "Drift tier disabled",
        }
    }

    /// Build a wire-ready [`RpcErrorBody`] for this variant. Optional
    /// `data` is forwarded as-is.
    pub fn body(self, data: Option<serde_json::Value>) -> RpcErrorBody {
        RpcErrorBody {
            code: self.code(),
            message: self.message().to_owned(),
            data,
        }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RPC {}: {}", self.code(), self.message())
    }
}

impl std::error::Error for RpcError {}

/// Encoding / decoding helpers around a [`UnixStream`]. All framing
/// helpers are async and tolerate partial reads.
#[derive(Debug)]
pub struct UnixSocket;

impl UnixSocket {
    /// Write a single frame: 4-byte BE length followed by `bytes`.
    pub async fn write_frame<W: AsyncWrite + Unpin>(
        writer: &mut W,
        bytes: &[u8],
    ) -> std::io::Result<()> {
        let len = u32::try_from(bytes.len()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame too large")
        })?;
        if len > MAX_FRAME_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "frame exceeds MAX_FRAME_BYTES",
            ));
        }
        writer.write_all(&len.to_be_bytes()).await?;
        writer.write_all(bytes).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Read one frame. Returns `Ok(None)` on clean EOF.
    pub async fn read_frame<R: AsyncRead + Unpin>(
        reader: &mut R,
    ) -> std::io::Result<Option<Vec<u8>>> {
        let mut header = [0u8; 4];
        match reader.read_exact(&mut header).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let len = u32::from_be_bytes(header);
        if len > MAX_FRAME_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("frame length {len} exceeds MAX_FRAME_BYTES"),
            ));
        }
        let mut buf = vec![0u8; len as usize];
        reader.read_exact(&mut buf).await?;
        Ok(Some(buf))
    }

    /// Convenience: write a JSON-encoded [`RpcMessage`].
    pub async fn write_message<W: AsyncWrite + Unpin>(
        writer: &mut W,
        msg: &RpcMessage,
    ) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(msg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Self::write_frame(writer, &bytes).await
    }

    /// Convenience: read a JSON-encoded [`RpcMessage`].
    pub async fn read_message<R: AsyncRead + Unpin>(
        reader: &mut R,
    ) -> std::io::Result<Option<RpcMessage>> {
        match Self::read_frame(reader).await? {
            Some(bytes) => {
                let msg = serde_json::from_slice(&bytes).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
                })?;
                Ok(Some(msg))
            }
            None => Ok(None),
        }
    }

    /// Connect to a UDS at `path`. Convenience wrapper used by tests
    /// and any in-process client.
    pub async fn connect<P: AsRef<Path>>(path: P) -> std::io::Result<BufWriter<UnixStream>> {
        let stream = UnixStream::connect(path).await?;
        Ok(BufWriter::new(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn round_trip_frame() {
        let (a, mut b) = duplex(64 * 1024);
        let payload = b"hello world";
        UnixSocket::write_frame(&mut b, payload).await.expect("write");
        drop(b);
        let mut a = a;
        let frame = UnixSocket::read_frame(&mut a).await.expect("read");
        assert_eq!(frame.as_deref(), Some(payload.as_ref()));
    }

    #[tokio::test]
    async fn round_trip_message() {
        let (a, mut b) = duplex(64 * 1024);
        let msg = RpcMessage::Success(RpcSuccess {
            jsonrpc: "2.0".to_owned(),
            result: serde_json::json!({"hello": "world"}),
            id: serde_json::json!(1),
        });
        UnixSocket::write_message(&mut b, &msg).await.expect("write");
        drop(b);
        let mut a = a;
        let decoded = UnixSocket::read_message(&mut a).await.expect("read");
        match decoded {
            Some(RpcMessage::Success(s)) => {
                assert_eq!(s.id, serde_json::json!(1));
                assert_eq!(s.result["hello"], "world");
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn clean_eof_returns_none() {
        let (a, b) = duplex(64);
        drop(b);
        let mut a = a;
        let frame = UnixSocket::read_frame(&mut a).await.expect("read");
        assert!(frame.is_none());
    }

    #[test]
    fn rpc_error_codes_match_spec() {
        assert_eq!(RpcError::ParseError.code(), -32700);
        assert_eq!(RpcError::InvalidRequest.code(), -32600);
        assert_eq!(RpcError::MethodNotFound.code(), -32601);
        assert_eq!(RpcError::InvalidParams.code(), -32602);
        assert_eq!(RpcError::InternalError.code(), -32603);
        assert_eq!(RpcError::DaemonUnavail.code(), -32001);
        assert_eq!(RpcError::DriftTierDisabled.code(), -32002);
    }

    #[test]
    fn rpc_error_body_round_trips() {
        let body = RpcError::MethodNotFound.body(Some(serde_json::json!({"method": "x"})));
        assert_eq!(body.code, -32601);
        assert_eq!(body.message, "Method not found");
        assert_eq!(body.data.unwrap()["method"], "x");
    }

    #[test]
    fn notify_envelope_serializes_with_method() {
        let env = NotifyEnvelope {
            jsonrpc: "2.0".to_owned(),
            method: NOTIFY_METHOD.to_owned(),
            params: NotifyParams {
                topic: "drift.alert".to_owned(),
                payload: serde_json::json!({"id": 1}),
            },
        };
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["method"], "notify");
        assert_eq!(v["params"]["topic"], "drift.alert");
    }
}