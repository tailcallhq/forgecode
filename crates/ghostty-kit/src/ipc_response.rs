//! Incoming-side helpers: read a length-prefixed frame, parse the
//! response envelope, and project typed payloads ([`WindowSize`]) out
//! of the raw `data` field.
//!
//! Split out of `ipc.rs` to keep the main module under the workspace's
//! 500-LOC ceiling. Functions here are `pub(crate)` and are not part of
//! the crate's stable surface.

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::ipc::{IpcError, JsonValue, Response, WindowSize};

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Read a single length-prefixed JSON response from `stream`.
///
/// Maps `TimedOut` to [`IpcError::Timeout`], `UnexpectedEof` to
/// [`IpcError::ConnectionLost`], and any other I/O failure to
/// [`IpcError::ConnectionLost`] with the underlying error stringified.
pub(crate) fn read_framed_response(
    stream: &mut UnixStream,
    timeout: Duration,
) -> Result<Response, IpcError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).map_err(|e| match e.kind() {
        std::io::ErrorKind::TimedOut => IpcError::Timeout(timeout),
        std::io::ErrorKind::UnexpectedEof => {
            IpcError::ConnectionLost("peer closed before sending length".to_owned())
        }
        _ => IpcError::ConnectionLost(format!("read length: {e}")),
    })?;
    let len = u32::from_be_bytes(len_buf);
    if len > 16 * 1024 * 1024 {
        return Err(IpcError::Protocol(format!(
            "response length {len} exceeds 16 MiB cap"
        )));
    }
    let mut body = vec![0u8; len as usize];
    stream.read_exact(&mut body).map_err(|e| match e.kind() {
        std::io::ErrorKind::TimedOut => IpcError::Timeout(timeout),
        std::io::ErrorKind::UnexpectedEof => {
            IpcError::ConnectionLost("peer closed before sending body".to_owned())
        }
        _ => IpcError::ConnectionLost(format!("read body: {e}")),
    })?;
    let text = std::str::from_utf8(&body)
        .map_err(|e| IpcError::Protocol(format!("response is not valid UTF-8: {e}")))?;
    parse_response(text)
}

// ---------------------------------------------------------------------------
// Response envelope
// ---------------------------------------------------------------------------

/// Parse a server response. Tolerates a missing `data` field on the
/// success path and a missing `error` field on the failure path.
/// Unknown fields are silently dropped for forward compatibility.
pub(crate) fn parse_response(text: &str) -> Result<Response, IpcError> {
    let bytes = text.as_bytes();
    let (obj_start, obj_end) = find_object_bounds(bytes)
        .ok_or_else(|| IpcError::Protocol("response is not a JSON object".to_owned()))?;
    let inner = &text[obj_start + 1..obj_end];

    let mut ok: Option<bool> = None;
    let mut data: Option<JsonValue> = None;
    let mut error: Option<String> = None;
    let mut rest = inner;
    while let Some(field) = next_field(rest) {
        let (key, value_start, value_end, after) = field;
        // next_field returns indices relative to `rest`, not `inner`.
        let value_src = &rest[value_start..value_end];
        match key {
            "ok" => {
                ok = Some(parse_bool_strict(value_src).ok_or_else(|| {
                    IpcError::Protocol(format!("'ok' is not a boolean: {value_src}"))
                })?);
            }
            "data" => {
                data = Some(
                    parse_json_value(value_src)
                        .ok_or_else(|| IpcError::Protocol("'data' is not valid JSON".to_owned()))?,
                );
            }
            "error" => {
                if let Some(JsonValue::String(s)) = parse_json_value(value_src) {
                    error = Some(s);
                } else {
                    return Err(IpcError::Protocol(format!(
                        "'error' is not a string: {value_src}"
                    )));
                }
            }
            _ => {
                // Unknown fields are tolerated for forward compat.
            }
        }
        rest = after;
    }
    let ok = ok.ok_or_else(|| IpcError::Protocol("response missing 'ok' field".to_owned()))?;
    Ok(Response { ok, data, error })
}

// ---------------------------------------------------------------------------
// Typed payload projection
// ---------------------------------------------------------------------------

/// Project a `get_window_size` response into a [`WindowSize`]. The
/// server is expected to reply with `{"ok": true, "data": [w, h, cw,
/// ch]}`. Anything else is reported as a protocol error.
pub(crate) fn parse_window_size(response: &Response) -> Result<WindowSize, IpcError> {
    let data = response.data.as_ref().ok_or_else(|| {
        IpcError::Protocol("get_window_size: response missing 'data'".to_owned())
    })?;
    let JsonValue::Array(parts) = data else {
        return Err(IpcError::Protocol(
            "get_window_size: expected array payload".to_owned(),
        ));
    };
    if parts.len() != 4 {
        return Err(IpcError::Protocol(format!(
            "get_window_size: expected 4 numbers, got {}",
            parts.len()
        )));
    }
    let mut nums = [0u16; 4];
    for (i, v) in parts.iter().enumerate() {
        nums[i] = match v {
            JsonValue::Int(n) if (0..=u16::MAX as i64).contains(n) => *n as u16,
            _ => {
                return Err(IpcError::Protocol(format!(
                    "get_window_size: field {i} is not a u16"
                )));
            }
        };
    }
    Ok(WindowSize {
        width: nums[0],
        height: nums[1],
        cell_width: nums[2],
        cell_height: nums[3],
    })
}

// ---------------------------------------------------------------------------
// JSON value parser
// ---------------------------------------------------------------------------

/// Parse any JSON value (string, integer, array, or literal) into a
/// [`JsonValue`]. Returns `None` on malformed input.
fn parse_json_value(src: &str) -> Option<JsonValue> {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    match bytes[0] {
        b'"' => parse_string_value(trimmed).map(JsonValue::String),
        b'[' => parse_array_value(trimmed),
        b't' if trimmed == "true" => Some(JsonValue::Bool(true)),
        b'f' if trimmed == "false" => Some(JsonValue::Bool(false)),
        b'n' if trimmed == "null" => Some(JsonValue::Null),
        b'0'..=b'9' | b'-' => parse_int_value(trimmed).map(JsonValue::Int),
        _ => None,
    }
}

pub(crate) fn parse_string_value(src: &str) -> Option<String> {
    let bytes = src.as_bytes();
    if bytes.first()? != &b'"' || bytes.last()? != &b'"' {
        return None;
    }
    let inner = &src[1..src.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'b' => out.push('\x08'),
                'f' => out.push('\x0c'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let code = u32::from_str_radix(&hex, 16).ok()?;
                    let ch = char::from_u32(code)?;
                    out.push(ch);
                }
                _ => return None,
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

fn parse_array_value(src: &str) -> Option<JsonValue> {
    let bytes = src.as_bytes();
    if bytes.first()? != &b'[' || bytes.last()? != &b']' {
        return None;
    }
    let inner = &src[1..src.len() - 1];
    let mut items = Vec::new();
    let mut rest = inner;
    while !rest.trim().is_empty() {
        let trimmed = rest.trim_start();
        let end = scan_one_value(trimmed.as_bytes(), 0)?;
        let value_src = &trimmed[..end];
        items.push(parse_json_value(value_src)?);
        rest = &trimmed[end..];
    }
    Some(JsonValue::Array(items))
}

fn parse_int_value(src: &str) -> Option<i64> {
    src.parse::<i64>().ok()
}

fn parse_bool_strict(src: &str) -> Option<bool> {
    match src.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Field-walker used by parse_response
// ---------------------------------------------------------------------------

/// Pull the next `"key": value` field off the front of `src`.
/// Returns `(key, value_start_offset_in_full, value_end_offset_in_full,
/// remainder_after_value)`. The `value_start` / `value_end` indices are
/// relative to the *full* input `src` (i.e. they include any
/// already-consumed leading bytes), so the caller can use them to slice
/// out the value text from its own copy of the input.
fn next_field(src: &str) -> Option<(&str, usize, usize, &str)> {
    let bytes = src.as_bytes();
    let mut i = 0;
    // Skip leading whitespace and commas.
    while i < bytes.len() && (bytes[i] == b',' || bytes[i].is_ascii_whitespace()) {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    if bytes[i] != b'"' {
        return None;
    }
    let key_start = i + 1;
    let mut j = key_start;
    let mut escape = false;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' if !escape => {
                escape = true;
                j += 1;
            }
            b'"' if !escape => break,
            _ => {
                escape = false;
                j += 1;
            }
        }
    }
    if j >= bytes.len() {
        return None;
    }
    let key = &src[key_start..j];
    j += 1;
    // Skip whitespace and the colon.
    while j < bytes.len() && (bytes[j].is_ascii_whitespace() || bytes[j] == b':') {
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let value_start = j;
    let value_end = scan_one_value(bytes, j)?;
    let remainder = &src[value_end..];
    Some((key, value_start, value_end, remainder))
}

/// Return the index just past the end of the JSON value that starts at
/// `start` in `bytes`. Used by [`next_field`] to size the value slice.
fn scan_one_value(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    // Skip leading whitespace.
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let b = bytes[i];
    match b {
        b'"' => {
            let mut j = i + 1;
            let mut escape = false;
            while j < bytes.len() {
                match bytes[j] {
                    b'\\' if !escape => {
                        escape = true;
                        j += 1;
                    }
                    b'"' if !escape => return Some(j + 1),
                    _ => {
                        escape = false;
                        j += 1;
                    }
                }
            }
            None
        }
        b'{' => {
            let (s, e) = find_object_bounds(bytes.get(i..)?)?;
            Some(i + e + 1 - s + i)
        }
        b'[' => {
            let mut depth: i32 = 0;
            let mut in_string = false;
            let mut escape = false;
            for (k, b) in bytes.iter().enumerate().skip(i) {
                if escape {
                    escape = false;
                    continue;
                }
                match *b {
                    b'\\' if in_string => escape = true,
                    b'"' => in_string = !in_string,
                    b'[' if !in_string => depth += 1,
                    b']' if !in_string => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(k + 1);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        _ => {
            // Literal / number: read until comma, brace, or whitespace.
            let mut j = i;
            while j < bytes.len() {
                let b = bytes[j];
                if b == b',' || b == b'}' || b == b']' || b.is_ascii_whitespace() {
                    break;
                }
                j += 1;
            }
            Some(j)
        }
    }
}

/// Find the span of a top-level JSON object in `bytes` (assumed to be
/// valid UTF-8). Returns `(start, end)` of the outer braces.
fn find_object_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
    let start = bytes.iter().position(|b| *b == b'{')?;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    for (i, b) in bytes.iter().enumerate().skip(start) {
        if escape {
            escape = false;
            continue;
        }
        match *b {
            b'\\' if in_string => escape = true,
            b'"' => in_string = !in_string,
            b'{' if !in_string => depth += 1,
            b'}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some((start, i));
                }
            }
            _ => {}
        }
    }
    None
}
