use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::Stream;

/// A simple SSE event parsed from a byte stream.
#[derive(Debug, Clone)]
pub struct SSEEvent {
    #[allow(dead_code)]
    pub event_type: Option<String>,
    pub data: String,
    #[allow(dead_code)]
    pub id: Option<String>,
}

impl SSEEvent {
    /// Get the event data.
    pub fn data(&self) -> &str {
        &self.data
    }
}

#[derive(Debug, thiserror::Error)]
enum SseParserError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    #[error("Invalid UTF-8 in SSE stream")]
    InvalidUtf8 {
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("SSE stream read error")]
    Stream {
        #[source]
        source: E,
    },
}

/// A stream adapter that parses Server-Sent Events from a bytes stream.
///
/// This is a simple SSE parser for cases where we need to parse raw byte
/// streams (e.g., for providers that don't return proper Content-Type headers).
pub struct BytesToSSE<S, E> {
    inner: S,
    buffer: String,
    _phantom: std::marker::PhantomData<E>,
}

impl<S, E> BytesToSSE<S, E>
where
    S: Stream<Item = Result<Bytes, E>>,
{
    /// Create a new BytesToSSE stream from a bytes stream.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            buffer: String::new(),
            _phantom: std::marker::PhantomData,
        }
    }
}

// Implement Unpin when the inner stream is Unpin
impl<S: Unpin, E> Unpin for BytesToSSE<S, E> {}

impl<S, E> Stream for BytesToSSE<S, E>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    type Item = anyhow::Result<SSEEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // Try to find a complete event in the buffer (double newline marks end of
            // event)
            if let Some(pos) = self.buffer.find("\n\n") {
                let event_text = self.buffer[..pos].to_string();
                self.buffer = self.buffer[pos + 2..].to_string();

                // Parse the event
                let mut event_type = None;
                let mut data = String::new();
                let mut id = None;

                for line in event_text.lines() {
                    if let Some(colon_pos) = line.find(':') {
                        let field = &line[..colon_pos];
                        let value = line[colon_pos + 1..].trim_start();

                        match field {
                            "event" => event_type = Some(value.to_string()),
                            "data" => {
                                if !data.is_empty() {
                                    data.push('\n');
                                }
                                data.push_str(value);
                            }
                            "id" => id = Some(value.to_string()),
                            _ => {} // Ignore unknown fields
                        }
                    } else if !line.is_empty() {
                        // Line without colon is treated as data (some servers send this)
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(line);
                    }
                }

                return Poll::Ready(Some(Ok(SSEEvent { event_type, data, id })));
            }

            // Need more data - poll the inner stream
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => match String::from_utf8(bytes.to_vec()) {
                    Ok(text) => {
                        self.buffer.push_str(&text);
                    }
                    Err(e) => {
                        return Poll::Ready(Some(Err(SseParserError::<E>::InvalidUtf8 {
                            source: e,
                        }
                        .into())));
                    }
                },
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(
                        Err(SseParserError::<E>::Stream { source: e }.into()),
                    ));
                }
                Poll::Ready(None) => {
                    // Stream ended - if there's remaining data, return it as an event
                    if !self.buffer.trim().is_empty() {
                        let remaining = std::mem::take(&mut self.buffer);
                        // Try to parse the remaining buffer
                        let mut event_type = None;
                        let mut data = String::new();
                        let mut id = None;

                        for line in remaining.lines() {
                            if let Some(colon_pos) = line.find(':') {
                                let field = &line[..colon_pos];
                                let value = line[colon_pos + 1..].trim_start();

                                match field {
                                    "event" => event_type = Some(value.to_string()),
                                    "data" => {
                                        if !data.is_empty() {
                                            data.push('\n');
                                        }
                                        data.push_str(value);
                                    }
                                    "id" => id = Some(value.to_string()),
                                    _ => {}
                                }
                            }
                        }

                        if !data.is_empty() || event_type.is_some() {
                            return Poll::Ready(Some(Ok(SSEEvent { event_type, data, id })));
                        }
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}

/// Parse a byte stream as Server-Sent Events.
pub fn parse_sse_stream<S>(stream: S) -> BytesToSSE<S, reqwest::Error>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    BytesToSSE::new(stream)
}
