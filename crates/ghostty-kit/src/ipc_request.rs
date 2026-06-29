//! Outgoing-side helpers: framing, request envelope, and the small JSON
//! writer we use to serialize [`crate::ipc::JsonObject`] into wire
//! payloads.
//!
//! Split out of `ipc.rs` to keep the main module under the workspace's
//! 500-LOC ceiling. The functions here are `pub(crate)` and are not
//! part of the crate's stable surface.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::ipc::JsonValue;

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

/// Encode one request as a 4-byte big-endian length prefix followed by
/// the JSON payload. The receiver reads the length, then exactly that
/// many bytes.
pub(crate) fn frame_request(payload: &str) -> Vec<u8> {
    let len = u32::try_from(payload.len()).expect("request payload fits in u32");
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload.as_bytes());
    out
}

// ---------------------------------------------------------------------------
// Request envelope
// ---------------------------------------------------------------------------

/// Build the JSON request envelope `{"action": "...", "args": {...},
/// "id": "..."}` from a verb and its argument object.
pub(crate) fn build_request(action: &str, args: &JsonObject) -> String {
    let id = next_request_id();
    let mut s = String::with_capacity(64 + action.len() + args.serialized_len());
    s.push_str("{\"action\":");
    push_json_string(&mut s, action);
    s.push_str(",\"args\":");
    args.serialize_into(&mut s);
    s.push_str(",\"id\":");
    push_json_string(&mut s, &id);
    s.push('}');
    s
}

/// Monotonic counter used to make request IDs unique within a single
/// process. The `uuid` crate is intentionally not pulled in: the
/// PR-1 contract for `ghostty-kit` is "zero new heavy deps".
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_request_id() -> String {
    let n = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("{pid:08x}{n:08x}")
}

// ---------------------------------------------------------------------------
// Ordered JSON object
// ---------------------------------------------------------------------------

/// An ordered collection of key/value pairs used to assemble the
/// `args` object of a request. We preserve insertion order so the wire
/// format stays stable across runs (helpful for golden-style debugging).
#[derive(Debug, Clone, Default)]
pub(crate) struct JsonObject {
    entries: Vec<(String, JsonValue)>,
}

impl JsonObject {
    /// Create an empty object.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Insert a key/value pair and return `self` for chaining.
    pub(crate) fn insert(mut self, key: &str, value: JsonValue) -> Self {
        self.entries.push((key.to_owned(), value));
        self
    }

    /// Length in characters of the serialized form, used to size the
    /// request buffer.
    pub(crate) fn serialized_len(&self) -> usize {
        let mut len = 2; // braces
        for (i, (k, v)) in self.entries.iter().enumerate() {
            if i > 0 {
                len += 1; // comma
            }
            len += 2 + k.len() + k.chars().filter(|c| *c == '"' || *c == '\\').count();
            len += 1; // ':'
            len += json_value_serialized_len(v);
        }
        len
    }

    /// Serialize into an existing `String`.
    pub(crate) fn serialize_into(&self, out: &mut String) {
        out.push('{');
        for (i, (k, v)) in self.entries.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            push_json_string(out, k);
            out.push(':');
            push_json_value(out, v);
        }
        out.push('}');
    }
}

// ---------------------------------------------------------------------------
// JSON writer helpers
// ---------------------------------------------------------------------------

fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_json_value(out: &mut String, v: &JsonValue) {
    match v {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(true) => out.push_str("true"),
        JsonValue::Bool(false) => out.push_str("false"),
        JsonValue::Int(n) => {
            use std::fmt::Write as _;
            let _ = write!(out, "{n}");
        }
        JsonValue::String(s) => push_json_string(out, s),
        JsonValue::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_json_value(out, item);
            }
            out.push(']');
        }
        JsonValue::Object(entries) => {
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_json_string(out, k);
                out.push(':');
                push_json_value(out, v);
            }
            out.push('}');
        }
    }
}

fn json_value_serialized_len(v: &JsonValue) -> usize {
    match v {
        JsonValue::Null => 4,
        JsonValue::Bool(true) => 4,
        JsonValue::Bool(false) => 5,
        JsonValue::Int(n) => n.to_string().len(),
        JsonValue::String(s) => 2 + s.len() + s.chars().filter(|c| *c == '"' || *c == '\\').count(),
        JsonValue::Array(items) => {
            let mut len = 2; // brackets
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    len += 1;
                }
                len += json_value_serialized_len(item);
            }
            len
        }
        JsonValue::Object(entries) => {
            let mut len = 2; // braces
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    len += 1; // comma
                }
                len += 2 + k.len() + k.chars().filter(|c| *c == '"' || *c == '\\').count();
                len += 1; // ':'
                len += json_value_serialized_len(v);
            }
            len
        }
    }
}
