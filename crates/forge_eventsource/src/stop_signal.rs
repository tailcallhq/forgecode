/// Returns `true` when an SSE event data field signals the end of the stream.
///
/// Two terminal markers are recognised:
/// * `"[DONE]"` — the OpenAI / Anthropic sentinel that every provider using
///   the OpenAI-compatible SSE framing emits as the final data field.
/// * `""` — an empty data field that some providers emit as a keepalive or
///   implicit end-of-stream marker and that must be swallowed rather than
///   forwarded as a message.
///
/// # Usage
///
/// ```rust
/// use forge_eventsource::is_sse_terminal;
///
/// assert!(is_sse_terminal("[DONE]"));
/// assert!(is_sse_terminal(""));
/// assert!(!is_sse_terminal(r#"{"id":"1"}"#));
/// ```
pub fn is_sse_terminal(data: &str) -> bool {
    matches!(data, "[DONE]" | "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_sentinel_is_terminal() {
        assert!(is_sse_terminal("[DONE]"));
    }

    #[test]
    fn empty_string_is_terminal() {
        assert!(is_sse_terminal(""));
    }

    #[test]
    fn json_payload_is_not_terminal() {
        assert!(!is_sse_terminal(
            r#"{"choices":[{"delta":{"content":"hi"}}]}"#
        ));
    }

    #[test]
    fn partial_done_is_not_terminal() {
        assert!(!is_sse_terminal("[DONE"));
        assert!(!is_sse_terminal("DONE]"));
        assert!(!is_sse_terminal(" [DONE] "));
    }
}
