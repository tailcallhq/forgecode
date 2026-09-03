use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{Forge3Error, Result};

/// Maximum size of a single framed JSON-RPC payload (4 MiB).
///
/// Anything larger is almost certainly a bug or attack; we reject early rather
/// than allocating unbounded buffers. 4 MiB is plenty for the largest drift
/// observations we expect to ship.
pub const MAX_FRAME_BYTES: u32 = 4 * 1024 * 1024;

/// JSON-RPC 2.0 request envelope. We accept the standard fields plus a
/// `params` object; extension methods are namespaced with a `.` (e.g.
/// `drift.observe`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Request {
    /// Build a notification (no `id`); the server will not respond.
    pub fn notification(method: impl Into<String>, params: serde_json::Value) -> Self {
        Request {
            jsonrpc: "2.0".into(),
            method: method.into(),
            id: None,
            params,
        }
    }
}

/// JSON-RPC 2.0 success response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub jsonrpc: String,
    pub result: serde_json::Value,
    pub id: serde_json::Value,
}

/// JSON-RPC 2.0 error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub jsonrpc: String,
    pub error: ErrorBody,
    pub id: serde_json::Value,
}

/// Body of an [`ErrorResponse`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: i32,
    pub message: String,
}

/// Either a success or error reply. The server always emits exactly one of
/// these for every request that carries an `id`.
#[derive(Debug, Clone)]
pub enum Response {
    Success(SuccessResponse),
    Error(ErrorResponse),
}

impl Response {
    /// Serialize the response as a JSON-RPC envelope.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Response::Success(s) => serde_json::to_value(s).expect("always serializable"),
            Response::Error(e) => serde_json::to_value(e).expect("always serializable"),
        }
    }
}

/// Encode a JSON payload into the wire frame: 4-byte big-endian length header
/// followed by UTF-8 bytes.
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Decode a single frame from an async reader.
///
/// Returns `Ok(None)` if the peer closed cleanly before sending a length
/// header (EOF). Returns `Err(FrameLength)` if the announced length exceeds
/// [`MAX_FRAME_BYTES`].
pub async fn decode_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Option<Vec<u8>>> {
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(header);
    if len > MAX_FRAME_BYTES {
        return Err(Forge3Error::FrameLength(len));
    }
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

/// Write a single frame to an async writer, flushing afterwards.
pub async fn write_frame<W: AsyncWriteExt + Unpin>(writer: &mut W, payload: &[u8]) -> Result<()> {
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Parse a raw JSON byte slice into a [`Request`], mapping JSON parse errors
/// to a [`Forge3Error::Protocol`].
pub fn parse_request(bytes: &[u8]) -> Result<Request> {
    let v: serde_json::Value = serde_json::from_slice(bytes)?;
    let req: Request = serde_json::from_value(v)
        .map_err(|e| Forge3Error::Protocol(format!("invalid json-rpc request: {e}")))?;
    if req.jsonrpc != "2.0" {
        return Err(Forge3Error::Protocol(format!(
            "jsonrpc field must be \"2.0\", got {:?}",
            req.jsonrpc
        )));
    }
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let payload = br#"{"jsonrpc":"2.0","method":"ping","id":1}"#;
        let framed = encode_frame(payload);
        assert_eq!(&framed[..4], &(payload.len() as u32).to_be_bytes());
        assert_eq!(&framed[4..], payload);
    }

    #[test]
    fn frame_size_in_header_matches() {
        let payload = b"x".repeat(1024);
        let framed = encode_frame(&payload);
        let header = u32::from_be_bytes([framed[0], framed[1], framed[2], framed[3]]);
        assert_eq!(header as usize, payload.len());
    }

    #[test]
    fn parse_request_accepts_minimal_envelope() {
        let raw = br#"{"jsonrpc":"2.0","method":"drift.observe","id":1}"#;
        let req = parse_request(raw).expect("parse");
        assert_eq!(req.method, "drift.observe");
        assert_eq!(req.id, Some(serde_json::json!(1)));
    }

    #[test]
    fn parse_request_rejects_wrong_version() {
        let raw = br#"{"jsonrpc":"1.0","method":"x","id":1}"#;
        let err = parse_request(raw).unwrap_err();
        assert!(matches!(err, Forge3Error::Protocol(_)));
    }

    #[test]
    fn parse_request_rejects_missing_method() {
        let raw = br#"{"jsonrpc":"2.0","id":1}"#;
        let err = parse_request(raw).unwrap_err();
        assert!(matches!(err, Forge3Error::Protocol(_)));
    }

    #[test]
    fn error_response_serializes_to_envelope_shape() {
        let resp = Response::Error(ErrorResponse {
            jsonrpc: "2.0".into(),
            error: ErrorBody { code: -32600, message: "bad".into() },
            id: serde_json::json!(7),
        });
        let v = resp.to_json();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["error"]["code"], -32600);
        assert_eq!(v["id"], 7);
    }

    #[test]
    fn success_response_carries_id_and_result() {
        let resp = Response::Success(SuccessResponse {
            jsonrpc: "2.0".into(),
            result: serde_json::json!({"ok": true}),
            id: serde_json::json!("abc"),
        });
        let v = resp.to_json();
        assert_eq!(v["result"]["ok"], true);
        assert_eq!(v["id"], "abc");
    }

    #[tokio::test]
    async fn decode_frame_returns_none_on_eof() {
        // Empty reader -> no header available -> Ok(None).
        let mut buf: &[u8] = &[];
        let out = decode_frame(&mut buf).await.expect("ok");
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn decode_frame_roundtrip_via_cursor() {
        let payload = b"hello world";
        let framed = encode_frame(payload);
        let mut cursor = &framed[..];
        let got = decode_frame(&mut cursor).await.expect("ok").expect("some");
        assert_eq!(got, payload);
    }

    #[tokio::test]
    async fn decode_frame_rejects_oversize_header() {
        // Build a frame whose header advertises > MAX_FRAME_BYTES.
        let header = (MAX_FRAME_BYTES + 1).to_be_bytes();
        let bytes: Vec<u8> = header.iter().chain(&[0u8; 0]).copied().collect();
        let mut cursor: &[u8] = &bytes;
        let err = decode_frame(&mut cursor).await.unwrap_err();
        assert!(matches!(err, Forge3Error::FrameLength(_)));
    }
}
