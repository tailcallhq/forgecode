use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use anyhow::Context as _;
use async_openai::types::responses as oai;
use forge_app::domain::{ChatCompletionMessage, ResultStream};
use forge_domain::BoxStream;
use futures::{SinkExt, StreamExt};
use reqwest::header::HeaderMap;
use thiserror::Error;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
#[cfg(test)]
use tokio_tungstenite::tungstenite::error::UrlError;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::debug;
use url::Url;

use crate::provider::IntoDomain;

type WebSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Per-conversation continuation state for the Responses WebSocket transport.
///
/// Holds the most recent response id, the input prefix that was visible to the
/// server when that response was generated, and (optionally) a still-open
/// socket carried over from the previous turn. Together these let a follow-up
/// turn send only delta input items + `previous_response_id` and skip a fresh
/// TLS handshake.
#[derive(Default)]
struct SessionState {
    previous_response_id: Option<String>,
    input_len: usize,
    input_signature: u64,
    socket: Option<WebSocket>,
}

#[derive(Clone, Default)]
pub(super) struct Session {
    state: Arc<Mutex<SessionState>>,
}

impl Session {
    /// Mutates `request` for continuation when the cached prefix is intact.
    ///
    /// On a hit, the request is rewritten to contain only the delta items
    /// (tool outputs and new user messages) plus `previous_response_id`. On a
    /// miss the cache is cleared and the request is left as a fresh turn.
    /// `store` is always set to `false` so the connection-local cache holds
    /// the response state without persisting on the server.
    pub(super) async fn prepare_request(
        &self,
        request: &mut oai::CreateResponse,
    ) -> anyhow::Result<()> {
        request.store = Some(false);

        let (prev_id, prev_len, prev_sig) = {
            let state = self.state.lock().await;
            let Some(prev_id) = state.previous_response_id.clone() else {
                return Ok(());
            };
            (prev_id, state.input_len, state.input_signature)
        };

        match input_prefix_signature(request, prev_len)? {
            Some(sig) if sig == prev_sig => {}
            _ => {
                self.clear().await;
                return Ok(());
            }
        }

        let Some(delta) = continuation_delta(request, prev_len) else {
            self.clear().await;
            return Ok(());
        };

        request.previous_response_id = Some(prev_id);
        request.input = oai::InputParam::Items(delta);
        request.instructions = None;
        request.prompt_cache_key = None;
        Ok(())
    }

    async fn take_socket(
        &self,
        websocket_url: &Url,
        headers: HeaderMap,
    ) -> anyhow::Result<(WebSocket, bool)> {
        if let Some(socket) = self.state.lock().await.socket.take() {
            return Ok((socket, true));
        }

        let request = websocket_request(websocket_url, headers)?;
        let (socket, response) = connect_async(request)
            .await
            .map_err(|error| map_connect_error(error, websocket_url))?;

        debug!(
            status = %response.status(),
            "Connected to OpenAI Responses WebSocket"
        );
        Ok((socket, false))
    }

    async fn complete_turn(
        &self,
        response_id: String,
        input_len: usize,
        input_signature: u64,
        socket: WebSocket,
    ) {
        let mut state = self.state.lock().await;
        state.previous_response_id = Some(response_id);
        state.input_len = input_len;
        state.input_signature = input_signature;
        state.socket = Some(socket);
    }

    pub(super) async fn clear(&self) {
        let mut state = self.state.lock().await;
        *state = SessionState::default();
    }

    #[cfg(test)]
    async fn seed_for_test(&self, response_id: String, input_len: usize, input_signature: u64) {
        let mut state = self.state.lock().await;
        state.previous_response_id = Some(response_id);
        state.input_len = input_len;
        state.input_signature = input_signature;
    }

    #[cfg(test)]
    async fn previous_response_id_for_test(&self) -> Option<String> {
        self.state.lock().await.previous_response_id.clone()
    }
}

/// Mutable state carried by the dispatcher loop while a single response is
/// streaming. After the loop exits this is consulted to decide whether to
/// stash the socket back on the session for reuse.
struct ActiveResponse {
    session: Session,
    input_len: usize,
    input_signature: u64,
    response_id: Option<String>,
    terminal_success: bool,
    terminated: bool,
}

/// Error emitted when the WebSocket cannot be opened or the initial
/// `response.create` event cannot be sent. This is the trigger for the HTTP
/// fallback path in `repository.rs` — mid-stream errors propagate as plain
/// `anyhow::Error`s instead.
#[derive(Debug, Error)]
#[error("OpenAI Responses WebSocket connect failed for {url}")]
pub(super) struct ConnectError {
    url: String,
    #[source]
    source: Option<anyhow::Error>,
}

impl ConnectError {
    fn with_source(url: &Url, source: anyhow::Error) -> Self {
        Self { url: url.to_string(), source: Some(source) }
    }
}

/// Streams a Responses API request over a WebSocket connection.
///
/// On success returns a stream of domain `ChatCompletionMessage`s emitted via
/// the same converter as the HTTP path. On a connect-time failure returns a
/// `ConnectError` so the caller can fall back to HTTP.
pub(super) async fn chat(
    responses_url: Url,
    headers: HeaderMap,
    request: oai::CreateResponse,
    session: Session,
    full_input_len: usize,
    input_signature: u64,
) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
    let websocket_url = websocket_url_from_responses_url(&responses_url)?;
    let create_event = response_create_event(&request)?;
    let (mut socket, reused) = session.take_socket(&websocket_url, headers).await?;
    debug!(reused, "Using OpenAI Responses WebSocket connection");

    if let Err(error) = socket.send(Message::Text(create_event.into())).await {
        session.clear().await;
        return Err(ConnectError::with_source(
            &websocket_url,
            anyhow::Error::from(error)
                .context("Failed to send OpenAI Responses WebSocket create event"),
        )
        .into());
    }

    let (tx, rx) = mpsc::channel::<anyhow::Result<super::response::StreamItem>>(64);
    let active = ActiveResponse {
        session,
        input_len: full_input_len,
        input_signature,
        response_id: None,
        terminal_success: false,
        terminated: false,
    };
    tokio::spawn(async move {
        if let Err(error) = run_websocket(socket, tx.clone(), active).await {
            let _ = tx.send(Err(error)).await;
        }
    });

    let stream: BoxStream<super::response::StreamItem, anyhow::Error> =
        Box::pin(ReceiverStream::new(rx));
    stream.into_domain()
}

async fn run_websocket(
    mut socket: WebSocket,
    tx: mpsc::Sender<anyhow::Result<super::response::StreamItem>>,
    mut active: ActiveResponse,
) -> anyhow::Result<()> {
    while let Some(message) = socket.next().await {
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                active.session.clear().await;
                return Err(anyhow::Error::from(error)
                    .context("OpenAI Responses WebSocket receive failed"));
            }
        };

        let parsed = match message {
            Message::Text(text) => parse_text_message(text.as_ref(), &mut active)?,
            Message::Binary(bytes) => {
                let text = std::str::from_utf8(bytes.as_ref()).with_context(|| {
                    "OpenAI Responses WebSocket sent invalid UTF-8 binary data"
                })?;
                parse_text_message(text, &mut active)?
            }
            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .with_context(|| "Failed to send OpenAI Responses WebSocket pong")?;
                continue;
            }
            Message::Pong(_) | Message::Frame(_) => continue,
            Message::Close(_) => break,
        };

        let Some(parsed) = parsed else { continue };

        if parsed.terminal {
            // Stash the warm socket back into the session cache *before*
            // forwarding the terminal event to the caller. Without this
            // ordering, the caller's stream sees the terminal item, decides
            // to start the next turn, and races against this task to put
            // the socket back — losing the race forces a fresh handshake.
            // Doing the stash first guarantees the next `take_socket()`
            // call sees the warm connection.
            if active.terminal_success {
                if let Some(response_id) = active.response_id.take() {
                    active
                        .session
                        .complete_turn(
                            response_id,
                            active.input_len,
                            active.input_signature,
                            socket,
                        )
                        .await;
                } else {
                    active.session.clear().await;
                }
            } else {
                active.session.clear().await;
            }

            // Receiver dropped is fine here — we're returning anyway.
            let _ = tx.send(Ok(parsed.item)).await;
            return Ok(());
        }

        tx.send(Ok(parsed.item))
            .await
            .map_err(|_| anyhow::anyhow!("OpenAI Responses WebSocket receiver dropped"))?;
    }

    // Socket closed without a terminal event — drop any cached state so the
    // next turn opens a fresh connection.
    active.session.clear().await;
    Ok(())
}

struct ParsedItem {
    item: super::response::StreamItem,
    terminal: bool,
}

fn parse_text_message(
    text: &str,
    active: &mut ActiveResponse,
) -> anyhow::Result<Option<ParsedItem>> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Ok(None);
    }

    let event: super::response::ResponsesStreamEvent = serde_json::from_str(trimmed)
        .with_context(|| format!("Failed to parse WebSocket event: {trimmed}"))?;

    let (item, terminal, terminal_success, response_id) = match event {
        super::response::ResponsesStreamEvent::Keepalive { .. }
        | super::response::ResponsesStreamEvent::Unknown(_) => return Ok(None),
        super::response::ResponsesStreamEvent::Ping { cost } => {
            let usage = forge_domain::Usage { cost: Some(cost), ..Default::default() };
            let item = super::response::StreamItem::Message(Box::new(
                ChatCompletionMessage::assistant(forge_domain::Content::part("")).usage(usage),
            ));
            (item, false, false, None)
        }
        super::response::ResponsesStreamEvent::Response(event) => {
            let response_id = response_id_from_event(&event);
            let success = matches!(
                *event,
                oai::ResponseStreamEvent::ResponseCompleted(_)
                    | oai::ResponseStreamEvent::ResponseIncomplete(_)
            );
            let terminal = success
                || matches!(
                    *event,
                    oai::ResponseStreamEvent::ResponseFailed(_)
                        | oai::ResponseStreamEvent::ResponseError(_)
                );
            (super::response::StreamItem::Event(event), terminal, success, response_id)
        }
    };

    if let Some(response_id) = response_id {
        active.response_id = Some(response_id);
    }
    if terminal {
        active.terminated = true;
        active.terminal_success = terminal_success;
    }

    Ok(Some(ParsedItem { item, terminal }))
}

fn response_id_from_event(event: &oai::ResponseStreamEvent) -> Option<String> {
    match event {
        oai::ResponseStreamEvent::ResponseCreated(event) => Some(event.response.id.clone()),
        oai::ResponseStreamEvent::ResponseInProgress(event) => Some(event.response.id.clone()),
        oai::ResponseStreamEvent::ResponseCompleted(event) => Some(event.response.id.clone()),
        oai::ResponseStreamEvent::ResponseIncomplete(event) => Some(event.response.id.clone()),
        _ => None,
    }
}

/// Converts an HTTP Responses endpoint into its WebSocket equivalent.
pub(super) fn websocket_url_from_responses_url(responses_url: &Url) -> anyhow::Result<Url> {
    let mut url = responses_url.clone();
    match responses_url.scheme() {
        "https" => url
            .set_scheme("wss")
            .map_err(|_| anyhow::anyhow!("failed to set WebSocket URL scheme"))?,
        "http" => url
            .set_scheme("ws")
            .map_err(|_| anyhow::anyhow!("failed to set WebSocket URL scheme"))?,
        scheme => {
            return Err(anyhow::anyhow!(
                "unsupported Responses WebSocket scheme: {scheme}"
            ));
        }
    }
    Ok(url)
}

/// Builds the `response.create` payload by serializing the request, stripping
/// transport-only fields that are implied by the WebSocket connection itself,
/// and tagging the event with `"type": "response.create"`.
pub(super) fn response_create_event(request: &oai::CreateResponse) -> anyhow::Result<String> {
    let mut value = serde_json::to_value(request)
        .with_context(|| "Failed to serialize OpenAI Responses WebSocket request")?;
    let object = value.as_object_mut().ok_or_else(|| {
        anyhow::anyhow!("OpenAI Responses WebSocket request must be a JSON object")
    })?;

    object.remove("stream_options");
    object.remove("background");
    object.remove("stream");
    object.insert(
        "type".to_string(),
        serde_json::Value::String("response.create".to_string()),
    );

    serde_json::to_string(&value)
        .with_context(|| "Failed to encode OpenAI Responses WebSocket event")
}

fn map_connect_error(error: WsError, websocket_url: &Url) -> anyhow::Error {
    let source = match error {
        WsError::Http(response) => {
            let status = response.status();
            anyhow::anyhow!("OpenAI Responses WebSocket handshake failed with HTTP {status}")
                .context(crate::provider::utils::format_http_context(
                    Some(status),
                    "GET",
                    websocket_url,
                ))
        }
        error => anyhow::Error::from(error),
    };
    ConnectError::with_source(websocket_url, source).into()
}

fn websocket_request(
    websocket_url: &Url,
    headers: HeaderMap,
) -> anyhow::Result<http::Request<()>> {
    let mut request = websocket_url
        .as_str()
        .into_client_request()
        .with_context(|| format!("Failed to build WebSocket request for {websocket_url}"))?;

    let request_headers = request.headers_mut();
    let mut last_name: Option<reqwest::header::HeaderName> = None;
    for (name, value) in headers {
        let name = match name {
            Some(name) => {
                last_name = Some(name.clone());
                name
            }
            None => match last_name.clone() {
                Some(name) => name,
                None => continue,
            },
        };
        request_headers.append(name, value);
    }
    Ok(request)
}

/// Computes a stable signature for the leading `len` items of the request
/// input. The signature is used to detect whether the conversation prefix
/// the cache observed is still intact.
pub(super) fn input_signature(
    request: &oai::CreateResponse,
    len: usize,
) -> anyhow::Result<u64> {
    let oai::InputParam::Items(items) = &request.input else {
        return Err(anyhow::anyhow!(
            "OpenAI Responses WebSocket requires Items input, found Text"
        ));
    };
    if len > items.len() {
        return Err(anyhow::anyhow!(
            "input prefix length {} exceeds total items {}",
            len,
            items.len()
        ));
    }
    let bytes = serde_json::to_vec(&items[..len])
        .with_context(|| "Failed to serialize OpenAI Responses input prefix")?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

fn input_prefix_signature(
    request: &oai::CreateResponse,
    len: usize,
) -> anyhow::Result<Option<u64>> {
    let oai::InputParam::Items(items) = &request.input else {
        return Ok(None);
    };
    if len > items.len() {
        return Ok(None);
    }
    let bytes = serde_json::to_vec(&items[..len])
        .with_context(|| "Failed to serialize OpenAI Responses input prefix")?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(Some(hasher.finish()))
}

/// Returns the items to send as continuation input.
///
/// In WebSocket continuation mode the server already has the prior turn's
/// output (text, tool calls, reasoning) attached to `previous_response_id`,
/// so the next turn must contain only items that are genuinely new from the
/// server's perspective: function-call outputs and user messages. Assistant
/// text, function-call requests, and reasoning items in the suffix are
/// dropped.
fn continuation_delta(
    request: &oai::CreateResponse,
    skip: usize,
) -> Option<Vec<oai::InputItem>> {
    let oai::InputParam::Items(items) = &request.input else {
        return None;
    };
    if skip > items.len() {
        return None;
    }

    let delta: Vec<oai::InputItem> =
        items[skip..].iter().filter(|item| is_continuation_item(item)).cloned().collect();
    if delta.is_empty() { None } else { Some(delta) }
}

fn is_continuation_item(item: &oai::InputItem) -> bool {
    match item {
        oai::InputItem::Item(oai::Item::FunctionCallOutput(_)) => true,
        oai::InputItem::EasyMessage(message) => message.role == oai::Role::User,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use async_openai::types::responses as oai;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use url::Url;

    use super::*;

    fn message(role: oai::Role, text: &str) -> oai::InputItem {
        oai::InputItem::EasyMessage(oai::EasyInputMessage {
            r#type: oai::MessageType::Message,
            role,
            content: oai::EasyInputContent::Text(text.to_string()),
            phase: None,
        })
    }

    fn function_call(call_id: &str) -> oai::InputItem {
        oai::InputItem::Item(oai::Item::FunctionCall(oai::FunctionToolCall {
            arguments: "{}".to_string(),
            call_id: call_id.to_string(),
            name: "noop".to_string(),
            namespace: None,
            id: None,
            status: None,
        }))
    }

    fn function_output(call_id: &str, output: &str) -> oai::InputItem {
        oai::InputItem::Item(oai::Item::FunctionCallOutput(oai::FunctionCallOutputItemParam {
            call_id: call_id.to_string(),
            output: oai::FunctionCallOutput::Text(output.to_string()),
            id: None,
            status: None,
        }))
    }

    fn create_response(items: Vec<oai::InputItem>) -> oai::CreateResponse {
        oai::CreateResponse {
            input: oai::InputParam::Items(items),
            instructions: Some("you are helpful".to_string()),
            prompt_cache_key: Some("conv".to_string()),
            stream: Some(true),
            ..Default::default()
        }
    }

    #[test]
    fn test_websocket_url_from_https_responses_url() {
        let fixture = Url::parse("https://api.openai.com/v1/responses").unwrap();
        let actual = websocket_url_from_responses_url(&fixture).unwrap();
        let expected = Url::parse("wss://api.openai.com/v1/responses").unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_websocket_url_from_http_responses_url() {
        let fixture = Url::parse("http://localhost:1234/v1/responses").unwrap();
        let actual = websocket_url_from_responses_url(&fixture).unwrap();
        let expected = Url::parse("ws://localhost:1234/v1/responses").unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_websocket_url_rejects_unsupported_scheme() {
        let fixture = Url::parse("file:///tmp/responses").unwrap();
        let actual = websocket_url_from_responses_url(&fixture).is_err();
        assert_eq!(actual, true);
    }

    #[test]
    fn test_response_create_event_strips_transport_fields() {
        let mut fixture = create_response(vec![message(oai::Role::User, "hello")]);
        fixture.background = Some(false);
        fixture.model = Some("gpt-5.5".to_string());

        let actual: serde_json::Value =
            serde_json::from_str(&response_create_event(&fixture).unwrap()).unwrap();

        let expected = json!({
            "type": "response.create",
            "model": "gpt-5.5",
            "input": [{"type": "message", "role": "user", "content": "hello"}],
            "instructions": "you are helpful",
            "prompt_cache_key": "conv",
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_map_connect_error_is_downcastable_to_connect_error() {
        let fixture_url = Url::parse("wss://api.openai.com/v1/responses").unwrap();
        let fixture = WsError::Url(UrlError::UnableToConnect("fixture".to_string()));

        let actual = map_connect_error(fixture, &fixture_url);

        let expected = true;
        assert_eq!(actual.downcast_ref::<ConnectError>().is_some(), expected);
    }

    #[test]
    fn test_websocket_request_preserves_authorization_header() {
        let fixture_url = Url::parse("wss://api.openai.com/v1/responses").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer test-key".parse().unwrap());

        let actual = websocket_request(&fixture_url, headers).unwrap();

        let expected = "Bearer test-key";
        assert_eq!(actual.headers().get("authorization").unwrap(), expected);
    }

    #[test]
    fn test_continuation_delta_keeps_user_messages_and_function_outputs() {
        let fixture = create_response(vec![
            message(oai::Role::User, "u1"),
            message(oai::Role::Assistant, "a1"),
            function_call("call_1"),
            function_output("call_1", "result"),
            message(oai::Role::User, "u2"),
        ]);

        let actual = continuation_delta(&fixture, 1).unwrap();
        let actual: Vec<serde_json::Value> =
            actual.iter().map(|item| serde_json::to_value(item).unwrap()).collect();

        let expected = vec![
            json!({
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "result",
            }),
            json!({
                "type": "message",
                "role": "user",
                "content": "u2",
            }),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_continuation_delta_returns_none_when_only_assistant_items_remain() {
        let fixture = create_response(vec![
            message(oai::Role::User, "u1"),
            message(oai::Role::Assistant, "a1"),
            function_call("call_1"),
        ]);

        let actual = continuation_delta(&fixture, 1).is_none();

        let expected = true;
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_prepare_request_uses_previous_response_id_and_delta_items() {
        let session = Session::default();
        let prefix = vec![message(oai::Role::User, "u1")];
        let prefix_sig = {
            let request = create_response(prefix.clone());
            input_signature(&request, 1).unwrap()
        };
        session.seed_for_test("resp_prev".to_string(), 1, prefix_sig).await;

        let mut request = create_response(vec![
            message(oai::Role::User, "u1"),
            message(oai::Role::Assistant, "a1"),
            function_output("call_1", "result"),
            message(oai::Role::User, "u2"),
        ]);

        session.prepare_request(&mut request).await.unwrap();

        let actual_input: serde_json::Value = serde_json::to_value(&request.input).unwrap();
        let expected_input = json!([
            {"type": "function_call_output", "call_id": "call_1", "output": "result"},
            {"type": "message", "role": "user", "content": "u2"},
        ]);
        assert_eq!(actual_input, expected_input);
        assert_eq!(request.previous_response_id.as_deref(), Some("resp_prev"));
        assert_eq!(request.instructions, None);
        assert_eq!(request.prompt_cache_key, None);
        assert_eq!(request.store, Some(false));
    }

    #[tokio::test]
    async fn test_prepare_request_clears_session_when_prefix_signature_differs() {
        let session = Session::default();
        session.seed_for_test("resp_prev".to_string(), 1, 0xdeadbeef).await;

        let mut request = create_response(vec![
            message(oai::Role::User, "u1"),
            message(oai::Role::User, "u2"),
        ]);

        session.prepare_request(&mut request).await.unwrap();

        assert_eq!(request.previous_response_id, None);
        assert_eq!(session.previous_response_id_for_test().await, None);
    }

    #[tokio::test]
    async fn test_prepare_request_clears_session_when_no_continuation_items() {
        let session = Session::default();
        let prefix = vec![message(oai::Role::User, "u1")];
        let prefix_sig = input_signature(&create_response(prefix.clone()), 1).unwrap();
        session.seed_for_test("resp_prev".to_string(), 1, prefix_sig).await;

        // Same prefix, but the only suffix is an assistant message — nothing
        // to continue with.
        let mut request = create_response(vec![
            message(oai::Role::User, "u1"),
            message(oai::Role::Assistant, "a1"),
        ]);

        session.prepare_request(&mut request).await.unwrap();

        assert_eq!(request.previous_response_id, None);
        assert_eq!(session.previous_response_id_for_test().await, None);
    }

    #[tokio::test]
    async fn test_prepare_request_without_prior_state_only_sets_store_false() {
        let session = Session::default();
        let mut request = create_response(vec![message(oai::Role::User, "u1")]);

        session.prepare_request(&mut request).await.unwrap();

        assert_eq!(request.previous_response_id, None);
        assert_eq!(request.store, Some(false));
        // Prefix and instructions are untouched on a fresh turn.
        assert_eq!(request.instructions.as_deref(), Some("you are helpful"));
    }
}
