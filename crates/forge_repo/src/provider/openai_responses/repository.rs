use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use async_openai::types::responses as oai;
use forge_app::domain::{
    ChatCompletionMessage, Context as ChatContext, Model, ModelId, ResultStream,
};
use forge_app::{EnvironmentInfra, HttpInfra};
use forge_domain::{BoxStream, ChatRepository, Provider};
use forge_eventsource_stream::Eventsource;
use forge_infra::sanitize_headers;
use futures::StreamExt;
use reqwest::StatusCode;
use reqwest::header::AUTHORIZATION;
use tracing::{info, warn};
use url::Url;

use crate::provider::FromDomain;
use crate::provider::retry::into_retry;
use crate::provider::utils::{create_headers, format_http_context, read_http_error_reason};
/// Type alias for the per-conversation WebSocket session cache shared by the
/// repository and its short-lived provider clients.
type WebSocketSessions = Arc<Mutex<HashMap<String, super::websocket::Session>>>;

#[derive(Clone)]
pub(super) struct OpenAIResponsesProvider<H> {
    provider: Provider<Url>,
    http: Arc<H>,
    api_base: Url,
    responses_url: Url,
    websocket_sessions: WebSocketSessions,
}

impl<H: HttpInfra> OpenAIResponsesProvider<H> {
    /// Creates a new OpenAI Responses provider
    ///
    /// For providers whose configured URL already points at a full Responses
    /// endpoint, the configured URL is used directly (for example,
    /// `chatgpt.com/backend-api/codex/responses`).
    /// For all other providers, the path is rewritten to `{host}/v1/responses`.
    ///
    /// # Panics
    ///
    /// Panics if the provider URL cannot be converted to an API base URL
    pub fn new(provider: Provider<Url>, http: Arc<H>) -> Self {
        Self::with_websocket_sessions(provider, http, WebSocketSessions::default())
    }

    /// Like [`OpenAIResponsesProvider::new`] but reuses an existing
    /// per-conversation WebSocket session cache. Used by the repository so
    /// successive `chat()` calls within the same process can share a warm
    /// socket and continuation state.
    pub fn with_websocket_sessions(
        provider: Provider<Url>,
        http: Arc<H>,
        websocket_sessions: WebSocketSessions,
    ) -> Self {
        use forge_domain::ProviderId;

        if provider.id == ProviderId::CODEX
            || provider.id == ProviderId::OPENCODE_ZEN
            || provider.id == ProviderId::OPENAI_RESPONSES_COMPATIBLE
        {
            // These providers already configure a complete Responses endpoint,
            // so preserve the configured path exactly as-is.
            let responses_url = provider.url.clone();
            let api_base = {
                let mut base = provider.url.clone();
                let path = base.path().trim_end_matches('/');
                let trimmed = path.strip_suffix("/responses").unwrap_or(path).to_owned();
                base.set_path(&trimmed);
                base.set_query(None);
                base.set_fragment(None);
                base
            };
            Self { provider, http, api_base, responses_url, websocket_sessions }
        } else {
            // Standard OpenAI pattern: rewrite to /v1/responses
            let api_base = api_base_from_endpoint_url(&provider.url)
                .expect("Failed to derive API base URL from provider endpoint");
            let responses_url = responses_endpoint_from_api_base(&api_base);
            Self { provider, http, api_base, responses_url, websocket_sessions }
        }
    }

    fn get_headers(&self) -> Vec<(String, String)> {
        self.get_headers_for_conversation(None)
    }

    fn get_headers_for_conversation(&self, conversation_id: Option<&str>) -> Vec<(String, String)> {
        let mut headers = Vec::new();
        if let Some(api_key) =
            self.provider
                .credential
                .as_ref()
                .and_then(|c| match &c.auth_details {
                    forge_domain::AuthDetails::ApiKey(key) => Some(key.as_str()),
                    forge_domain::AuthDetails::OAuthWithApiKey { api_key, .. } => {
                        Some(api_key.as_str())
                    }
                    forge_domain::AuthDetails::OAuth { tokens, .. } => {
                        Some(tokens.access_token.as_str())
                    }
                    forge_domain::AuthDetails::GoogleAdc(token) => Some(token.as_str()),
                    forge_domain::AuthDetails::AwsProfile(_) => None,
                })
        {
            headers.push((AUTHORIZATION.to_string(), format!("Bearer {api_key}")));
        }
        self.provider
            .auth_methods
            .iter()
            .for_each(|method| match method {
                forge_domain::AuthMethod::ApiKey => {}
                forge_domain::AuthMethod::OAuthDevice(oauth_config) => {
                    if let Some(custom_headers) = &oauth_config.custom_headers {
                        custom_headers.iter().for_each(|(k, v)| {
                            headers.push((k.clone(), v.clone()));
                        });
                    }
                }
                forge_domain::AuthMethod::OAuthCode(oauth_config) => {
                    if let Some(custom_headers) = &oauth_config.custom_headers {
                        custom_headers.iter().for_each(|(k, v)| {
                            headers.push((k.clone(), v.clone()));
                        });
                    }
                }
                forge_domain::AuthMethod::CodexDevice(oauth_config) => {
                    if let Some(custom_headers) = &oauth_config.custom_headers {
                        custom_headers.iter().for_each(|(k, v)| {
                            headers.push((k.clone(), v.clone()));
                        });
                    }
                }
                forge_domain::AuthMethod::GoogleAdc => {}
                forge_domain::AuthMethod::AwsProfile => {}
            });

        // Codex provider requires the ChatGPT-Account-Id header extracted
        // from the JWT at login.
        //
        // Mirror codex-rs conversation continuity headers by sending:
        // - x-client-request-id: conversation id
        // - session_id: conversation id
        if self.provider.id == forge_domain::ProviderId::CODEX {
            if let Some(conversation_id) = conversation_id {
                headers.push((
                    "x-client-request-id".to_string(),
                    conversation_id.to_string(),
                ));
                headers.push(("session_id".to_string(), conversation_id.to_string()));
            }

            // Add ChatGPT-Account-Id from credential's stored url_params.
            if let Some(account_id) = self.provider.credential.as_ref().and_then(|c| {
                let key: forge_domain::URLParam = "chatgpt_account_id".to_string().into();
                c.url_params.get(&key)
            }) {
                headers.push(("ChatGPT-Account-Id".to_string(), account_id.to_string()));
            }
        }

        headers
    }
}

impl<T: HttpInfra + EnvironmentInfra<Config = forge_config::ForgeConfig>>
    OpenAIResponsesProvider<T>
{
    pub async fn chat(
        &self,
        model: &ModelId,
        context: ChatContext,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let conversation_id = context.conversation_id.as_ref().map(ToString::to_string);
        let headers = create_headers(self.get_headers_for_conversation(conversation_id.as_deref()));
        let mut request = oai::CreateResponse::from_domain(context)?;
        request.model = Some(model.as_str().to_string());

        // Apply Codex-specific request adjustments via the transformer pipeline.
        if self.provider.id == forge_domain::ProviderId::CODEX {
            use forge_domain::Transformer;
            request = super::codex_transformer::CodexTransformer.transform(request);
        }
        let use_websocket = self.should_use_websocket()?;
        let websocket_request = if use_websocket {
            // Stash the full input length and signature *before* any delta
            // trimming so the session can record what the server has now
            // seen if the turn succeeds.
            let total_items = match &request.input {
                oai::InputParam::Items(items) => items.len(),
                oai::InputParam::Text(_) => 0,
            };
            if total_items == 0 {
                None
            } else {
                let signature = super::websocket::input_signature(&request, total_items)?;
                let session = conversation_id
                    .as_deref()
                    .map(|id| self.websocket_session(id))
                    .unwrap_or_default();
                let mut delta_request = request.clone();
                session.prepare_request(&mut delta_request).await?;
                Some((session, delta_request, total_items, signature))
            }
        } else {
            None
        };

        info!(
            url = %self.responses_url,
            base_url = %self.api_base,
            model = %model,
            headers = ?sanitize_headers(&headers),
            message_count = %request_message_count(&request),
            websocket = %use_websocket,
            "Connecting Upstream (Responses API)"
        );

        let json_bytes = serde_json::to_vec(&request)
            .with_context(|| "Failed to serialize OpenAI Responses request")?;

        if let Some((session, ws_request, total_items, signature)) = websocket_request {
            match super::websocket::chat(
                self.responses_url.clone(),
                headers.clone(),
                ws_request,
                session.clone(),
                total_items,
                signature,
            )
            .await
            {
                Ok(stream) => return Ok(stream),
                Err(error) if should_fallback_to_http(&error) => {
                    session.clear().await;
                    warn!(
                        error = %error,
                        "OpenAI Responses WebSocket unavailable; falling back to HTTP /responses"
                    );
                }
                Err(error) => return Err(error),
            }
        }


        // The Codex backend at chatgpt.com does not return
        // `Content-Type: text/event-stream`, which causes the
        // reqwest-eventsource library to reject the response with
        // `InvalidContentType`. We bypass it by making a direct HTTP POST
        // and parsing SSE from the raw byte stream using
        // eventsource-stream, exactly like the AI SDK does.
        if self.provider.id == forge_domain::ProviderId::CODEX {
            return self.chat_codex_stream(headers, json_bytes).await;
        }

        let source = self
            .http
            .http_eventsource(&self.responses_url, Some(headers), json_bytes.into())
            .await
            .with_context(|| format_http_context(None, "POST", &self.responses_url))?;

        // Parse SSE stream into domain messages and convert to domain type
        use forge_eventsource::Event;
        let url = self.responses_url.clone();
        let event_stream = source
            .take_while(|message| {
                let should_continue =
                    !matches!(message, Err(forge_eventsource::Error::StreamEnded));
                async move { should_continue }
            })
            .filter_map(move |event_result| {
                let url = url.clone();
                async move {
                    match event_result {
                        Ok(Event::Open) => None,
                        Ok(Event::Message(msg)) if ["[DONE]", ""].contains(&msg.data.as_str()) => {
                            None
                        }
                        Ok(Event::Message(msg)) => {
                            let result = serde_json::from_str::<
                                super::response::ResponsesStreamEvent,
                            >(&msg.data)
                            .with_context(|| format!("Failed to parse SSE event: {}", msg.data));

                            match result {
                                Ok(super::response::ResponsesStreamEvent::Keepalive { .. }) => None,
                                Ok(super::response::ResponsesStreamEvent::Ping { cost }) => {
                                    let usage = forge_domain::Usage {
                                        cost: Some(cost),
                                        ..Default::default()
                                    };
                                    Some(Ok(super::response::StreamItem::Message(Box::new(
                                        ChatCompletionMessage::assistant(
                                            forge_domain::Content::part(""),
                                        )
                                        .usage(usage),
                                    ))))
                                }
                                Ok(super::response::ResponsesStreamEvent::Unknown(_)) => None,
                                Ok(super::response::ResponsesStreamEvent::Response(inner)) => {
                                    Some(Ok(super::response::StreamItem::Event(inner)))
                                }
                                Err(e) => Some(Err(e)),
                            }
                        }
                        Err(forge_eventsource::Error::StreamEnded) => None,
                        Err(forge_eventsource::Error::InvalidStatusCode(_, response))
                        | Err(forge_eventsource::Error::InvalidContentType(_, response)) => {
                            let (_, reason) = read_http_error_reason(*response).await;
                            Some(Err(anyhow::anyhow!(reason)
                                .context(format_http_context(None, "POST", &url))))
                        }
                        Err(e) => {
                            Some(Err(anyhow::Error::from(e)
                                .context(format_http_context(None, "POST", &url))))
                        }
                    }
                }
            });

        // Convert to domain messages using the existing conversion logic
        use crate::provider::IntoDomain;
        let stream: BoxStream<super::response::StreamItem, anyhow::Error> = Box::pin(event_stream);
        stream.into_domain()
    }

    /// Streams a Codex chat response by making a direct HTTP POST and
    /// parsing SSE from the raw byte stream, bypassing Content-Type
    /// validation that `reqwest-eventsource` enforces.
    async fn chat_codex_stream(
        &self,
        headers: reqwest::header::HeaderMap,
        json_bytes: Vec<u8>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let response = self
            .http
            .http_post(&self.responses_url, Some(headers), json_bytes.into())
            .await
            .with_context(|| format_http_context(None, "POST", &self.responses_url))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read response body".to_string());
            return Err(status_code_error(status, error_body))
                .with_context(|| format_http_context(Some(status), "POST", &self.responses_url));
        }

        // Parse the raw byte stream as SSE events using eventsource-stream.
        // This mirrors the AI SDK approach: TextDecoderStream ->
        // EventSourceParserStream -> JSON parse, without any Content-Type
        // requirement.
        let byte_stream = response.bytes_stream();
        let event_stream = byte_stream
            .eventsource()
            .filter_map(|event_result| async move {
                match event_result {
                    Ok(event) if ["[DONE]", ""].contains(&event.data.as_str()) => None,
                    Ok(event) => {
                        let result = serde_json::from_str::<super::response::ResponsesStreamEvent>(
                            &event.data,
                        )
                        .with_context(|| format!("Failed to parse SSE event: {}", event.data));
                        match result {
                            Ok(super::response::ResponsesStreamEvent::Keepalive { .. }) => None,
                            Ok(super::response::ResponsesStreamEvent::Ping { cost }) => {
                                let usage =
                                    forge_domain::Usage { cost: Some(cost), ..Default::default() };
                                Some(Ok(super::response::StreamItem::Message(Box::new(
                                    ChatCompletionMessage::assistant(forge_domain::Content::part(
                                        "",
                                    ))
                                    .usage(usage),
                                ))))
                            }
                            Ok(super::response::ResponsesStreamEvent::Unknown(_)) => None,
                            Ok(super::response::ResponsesStreamEvent::Response(inner)) => {
                                Some(Ok(super::response::StreamItem::Event(inner)))
                            }
                            Err(e) => Some(Err(e)),
                        }
                    }
                    Err(e) => Some(Err(into_sse_parse_error(e))),
                }
            });

        use crate::provider::IntoDomain;
        let stream: BoxStream<super::response::StreamItem, anyhow::Error> = Box::pin(event_stream);
        stream.into_domain()
    }

    /// Returns the cached WebSocket session for `conversation_id`, creating a
    /// fresh one on first access. The map is held under a `std::sync::Mutex`
    /// because lookups are short and synchronous; per-session state lives
    /// behind a `tokio::Mutex` because it's held across `.await` points.
    fn websocket_session(&self, conversation_id: &str) -> super::websocket::Session {
        let mut sessions = self
            .websocket_sessions
            .lock()
            .expect("OpenAI Responses WebSocket session cache lock poisoned");
        sessions.entry(conversation_id.to_string()).or_default().clone()
    }

    /// Decides whether the WebSocket transport should be used for this turn.
    ///
    /// Resolution order:
    /// 1. The `GRAFF_OPENAI_RESPONSES_WEBSOCKET` env var (or the legacy
    ///    `FORGE_OPENAI_RESPONSES_WEBSOCKET`), if truthy, opts in.
    /// 2. Otherwise the persisted `openai_responses_websocket` config flag
    ///    decides.
    /// 3. Either way, the provider must speak the Responses-API protocol —
    ///    `OPENAI`, `OPENAI_RESPONSES_COMPATIBLE`, or `CODEX`. Codex hits
    ///    `chatgpt.com/backend-api/codex/responses`, which may or may not
    ///    accept the WebSocket upgrade; if it doesn't, the connect-time
    ///    error is mapped to `ConnectError` and `chat()` falls back to HTTP.
    fn should_use_websocket(&self) -> anyhow::Result<bool> {
        let env_enabled = self
            .http
            .get_env_var("GRAFF_OPENAI_RESPONSES_WEBSOCKET")
            .or_else(|| self.http.get_env_var("FORGE_OPENAI_RESPONSES_WEBSOCKET"))
            .is_some_and(|value| is_truthy(&value));
        let config_enabled = self.http.get_config()?.openai_responses_websocket;
        let provider_supported = self.provider.id == forge_domain::ProviderId::OPENAI
            || self.provider.id == forge_domain::ProviderId::OPENAI_RESPONSES_COMPATIBLE
            || self.provider.id == forge_domain::ProviderId::CODEX;
        Ok(provider_supported && (env_enabled || config_enabled))
    }
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn should_fallback_to_http(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|err| err.downcast_ref::<super::websocket::ConnectError>().is_some())
}

fn status_code_error(status: StatusCode, body: String) -> anyhow::Error {
    anyhow::Error::from(forge_app::dto::openai::Error::InvalidStatusCode(
        status.as_u16(),
    ))
    .context(body)
}

fn into_sse_parse_error<E>(error: forge_eventsource_stream::EventStreamError<E>) -> anyhow::Error
where
    E: std::fmt::Debug + std::fmt::Display + Send + Sync + 'static,
{
    let is_retryable = matches!(
        &error,
        forge_eventsource_stream::EventStreamError::Transport(_)
    );
    let error = anyhow::anyhow!("SSE parse error: {}", error);

    if is_retryable {
        forge_domain::Error::Retryable(error).into()
    } else {
        error
    }
}

/// Derives an API base URL suitable for OpenAI Responses API from a configured
/// endpoint URL.
///
/// For Codex/Responses usage we only need the host and the `/v1` prefix.
/// Any path on the incoming endpoint is ignored in favor of `/v1`.
fn api_base_from_endpoint_url(endpoint: &Url) -> anyhow::Result<Url> {
    let mut base = endpoint.clone();
    base.set_path("/v1");
    base.set_query(None);
    base.set_fragment(None);
    Ok(base)
}

fn responses_endpoint_from_api_base(api_base: &Url) -> Url {
    let mut url = api_base.clone();

    let mut path = api_base.path().trim_end_matches('/').to_string();
    path.push_str("/responses");

    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);

    url
}

fn request_message_count(request: &oai::CreateResponse) -> usize {
    match &request.input {
        oai::InputParam::Text(_) => 1,
        oai::InputParam::Items(items) => items.len(),
    }
}

/// Repository for OpenAI Codex models using the Responses API
///
/// Handles OpenAI's Codex models (e.g., gpt-5.1-codex, codex-mini-latest)
/// which use the Responses API instead of the standard Chat Completions API.
pub struct OpenAIResponsesResponseRepository<F> {
    infra: Arc<F>,
    websocket_sessions: WebSocketSessions,
}

impl<F> OpenAIResponsesResponseRepository<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra, websocket_sessions: WebSocketSessions::default() }
    }
}

#[async_trait::async_trait]
impl<F: HttpInfra + EnvironmentInfra<Config = forge_config::ForgeConfig> + 'static> ChatRepository
    for OpenAIResponsesResponseRepository<F>
{
    async fn chat(
        &self,
        model_id: &ModelId,
        context: ChatContext,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let retry_config = self.infra.get_config()?.retry.unwrap_or_default();
        let provider_client: OpenAIResponsesProvider<F> =
            OpenAIResponsesProvider::with_websocket_sessions(
                provider,
                self.infra.clone(),
                self.websocket_sessions.clone(),
            );
        let stream = provider_client
            .chat(model_id, context)
            .await
            .map_err(|e| into_retry(e, &retry_config))?;

        Ok(Box::pin(stream.map(move |item| {
            item.map_err(|e| into_retry(e, &retry_config))
        })))
    }

    async fn models(&self, provider: Provider<Url>) -> anyhow::Result<Vec<Model>> {
        match provider.models().cloned() {
            Some(forge_domain::ModelSource::Hardcoded(models)) => Ok(models),
            Some(forge_domain::ModelSource::Url(url)) => {
                let provider_client = OpenAIResponsesProvider::new(provider, self.infra.clone());
                let headers = create_headers(provider_client.get_headers());
                let response = self
                    .infra
                    .http_get(&url, Some(headers))
                    .await
                    .with_context(|| format_http_context(None, "GET", &url))
                    .with_context(|| "Failed to fetch models")?;

                let status = response.status();
                let ctx_message = format_http_context(Some(status), "GET", &url);
                let response_text = response
                    .text()
                    .await
                    .with_context(|| ctx_message.clone())
                    .with_context(|| "Failed to decode response into text")?;

                if !status.is_success() {
                    return Err(anyhow::anyhow!(response_text))
                        .with_context(|| ctx_message)
                        .with_context(|| "Failed to fetch models");
                }

                let data: forge_app::dto::openai::ListModelResponse =
                    serde_json::from_str(&response_text)
                        .with_context(|| format_http_context(None, "GET", &url))
                        .with_context(|| "Failed to deserialize models response")?;
                Ok(data.data.into_iter().map(Into::into).collect())
            }
            None => Ok(vec![]),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use forge_app::domain::{
        Content, Context as ChatContext, ContextMessage, FinishReason, ModelId, Provider,
        ProviderId, ProviderResponse,
    };
    use pretty_assertions::assert_eq;
    use tokio_stream::StreamExt;
    use url::Url;

    use super::*;
    use crate::provider::mock_server::MockServer;
    use crate::provider::retry;

    fn is_retryable(error: &anyhow::Error) -> bool {
        error
            .downcast_ref::<forge_domain::Error>()
            .is_some_and(|error| matches!(error, forge_domain::Error::Retryable(_)))
    }

    fn make_credential(provider_id: ProviderId, key: &str) -> Option<forge_domain::AuthCredential> {
        Some(forge_domain::AuthCredential {
            id: provider_id,
            auth_details: forge_domain::AuthDetails::ApiKey(forge_domain::ApiKey::from(
                key.to_string(),
            )),
            url_params: HashMap::new(),
        })
    }

    fn openai_responses(key: &str, url: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::OPENAI,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse(url).unwrap(),
            credential: make_credential(ProviderId::OPENAI, key),
            custom_headers: None,
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: None,
        }
    }

    /// Test fixture for creating a mock HTTP client.
    #[derive(Clone)]
    struct MockHttpClient {
        client: reqwest::Client,
    }

    #[async_trait::async_trait]
    impl HttpInfra for MockHttpClient {
        async fn http_get(
            &self,
            url: &reqwest::Url,
            headers: Option<reqwest::header::HeaderMap>,
        ) -> anyhow::Result<reqwest::Response> {
            let mut request = self.client.get(url.clone());
            if let Some(headers) = headers {
                request = request.headers(headers);
            }
            Ok(request.send().await?)
        }

        async fn http_post(
            &self,
            url: &reqwest::Url,
            headers: Option<reqwest::header::HeaderMap>,
            body: bytes::Bytes,
        ) -> anyhow::Result<reqwest::Response> {
            let mut request = self.client.post(url.clone()).body(body);
            if let Some(headers) = headers {
                request = request.headers(headers);
            }
            Ok(request.send().await?)
        }

        async fn http_delete(&self, _url: &reqwest::Url) -> anyhow::Result<reqwest::Response> {
            unimplemented!()
        }

        async fn http_eventsource(
            &self,
            url: &reqwest::Url,
            headers: Option<reqwest::header::HeaderMap>,
            body: bytes::Bytes,
        ) -> anyhow::Result<forge_eventsource::EventSource> {
            let mut request = self.client.post(url.clone()).body(body);
            if let Some(headers) = headers {
                request = request.headers(headers);
            }
            Ok(forge_eventsource::EventSource::new(request)?)
        }
    }

    impl forge_app::EnvironmentInfra for MockHttpClient {
        type Config = forge_config::ForgeConfig;

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
            std::collections::BTreeMap::new()
        }

        fn get_environment(&self) -> forge_domain::Environment {
            use fake::{Fake, Faker};
            Faker.fake()
        }

        fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
            Ok(forge_config::ForgeConfig::default())
        }

        async fn update_environment(
            &self,
            _ops: Vec<forge_domain::ConfigOperation>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    /// Test fixture for creating a sample OpenAI Responses API response.
    fn openai_response_fixture() -> serde_json::Value {
        serde_json::json!({
            "created_at": 0,
            "id": "resp_1",
            "model": "codex-mini-latest",
            "object": "response",
            "output": [{
                "type": "message",
                "id": "msg_1",
                "role": "assistant",
                "status": "completed",
                "content": [{
                    "type": "output_text",
                    "text": "hello",
                    "annotations": [],
                    "logprobs": null
                }]
            }],
            "status": "completed",
            "usage": {
                "input_tokens": 1,
                "output_tokens": 1,
                "total_tokens": 2,
                "input_tokens_details": {"cached_tokens": 0},
                "output_tokens_details": {"reasoning_tokens": 0}
            }
        })
    }

    #[test]
    fn test_status_code_error_preserves_retryable_status_code() {
        let fixture = StatusCode::SERVICE_UNAVAILABLE;

        let actual = status_code_error(fixture, "Connection refused".to_string());

        let expected = Some(503);
        assert_eq!(retry::get_api_status_code(&actual), expected);
    }

    #[test]
    fn test_status_code_error_preserves_body_context() {
        let fixture = "Connection refused".to_string();

        let actual = status_code_error(StatusCode::SERVICE_UNAVAILABLE, fixture.clone());

        let expected = true;
        assert_eq!(actual.to_string().contains(&fixture), expected);
    }

    #[test]
    fn test_api_base_from_endpoint_url_trims_expected_suffixes() -> anyhow::Result<()> {
        let openai_endpoint = Url::parse("https://api.openai.com/v1/chat/completions")?;
        let openai_base = api_base_from_endpoint_url(&openai_endpoint)?;
        assert_eq!(openai_base.as_str(), "https://api.openai.com/v1");

        let copilot_endpoint = Url::parse("https://api.githubcopilot.com/chat/completions")?;
        let copilot_base = api_base_from_endpoint_url(&copilot_endpoint)?;
        assert_eq!(copilot_base.as_str(), "https://api.githubcopilot.com/v1");

        Ok(())
    }

    #[test]
    fn test_api_base_from_endpoint_url_removes_query_and_fragment() -> anyhow::Result<()> {
        let url = Url::parse("https://api.openai.com/v1/path?query=1#fragment")?;
        let base = api_base_from_endpoint_url(&url)?;
        assert_eq!(base.as_str(), "https://api.openai.com/v1");
        assert!(base.query().is_none());
        assert!(base.fragment().is_none());

        Ok(())
    }

    #[test]
    fn test_responses_endpoint_from_api_base() -> anyhow::Result<()> {
        let api_base = Url::parse("https://api.openai.com/v1")?;
        let endpoint = responses_endpoint_from_api_base(&api_base);
        assert_eq!(endpoint.as_str(), "https://api.openai.com/v1/responses");

        let api_base = Url::parse("https://api.githubcopilot.com/v1/")?;
        let endpoint = responses_endpoint_from_api_base(&api_base);
        assert_eq!(
            endpoint.as_str(),
            "https://api.githubcopilot.com/v1/responses"
        );

        Ok(())
    }

    #[test]
    fn test_responses_endpoint_from_api_base_removes_query_and_fragment() -> anyhow::Result<()> {
        let api_base = Url::parse("https://api.openai.com/v1?query=1#fragment")?;
        let endpoint = responses_endpoint_from_api_base(&api_base);
        assert_eq!(endpoint.as_str(), "https://api.openai.com/v1/responses");
        assert!(endpoint.query().is_none());
        assert!(endpoint.fragment().is_none());

        Ok(())
    }

    #[test]
    fn test_request_message_count_with_text_input() {
        let request = oai::CreateResponse {
            input: oai::InputParam::Text("test".to_string()),
            ..Default::default()
        };
        assert_eq!(request_message_count(&request), 1);
    }

    #[test]
    fn test_request_message_count_with_items_input() {
        let request = oai::CreateResponse {
            input: oai::InputParam::Items(vec![
                oai::InputItem::Item(oai::Item::FunctionCall(oai::FunctionToolCall {
                    id: Some("call_1".to_string()),
                    call_id: "call_id_1".to_string(),
                    name: "tool1".to_string(),
                    arguments: "args1".to_string(),
                    namespace: None,
                    status: None,
                })),
                oai::InputItem::Item(oai::Item::FunctionCall(oai::FunctionToolCall {
                    id: Some("call_2".to_string()),
                    call_id: "call_id_2".to_string(),
                    name: "tool2".to_string(),
                    arguments: "args2".to_string(),
                    namespace: None,
                    status: None,
                })),
            ]),
            ..Default::default()
        };
        assert_eq!(request_message_count(&request), 2);
    }

    #[test]
    fn test_request_message_count_with_empty_items() {
        let request =
            oai::CreateResponse { input: oai::InputParam::Items(vec![]), ..Default::default() };
        assert_eq!(request_message_count(&request), 0);
    }

    #[test]
    fn test_openai_responses_provider_new_with_api_key() {
        let provider = openai_responses("test-key", "https://api.openai.com/v1");
        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);

        assert_eq!(provider_impl.api_base.as_str(), "https://api.openai.com/v1");
        assert_eq!(
            provider_impl.responses_url.as_str(),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn test_openai_responses_provider_new_preserves_existing_base_path_for_compatible_provider() {
        let provider = Provider {
            id: ProviderId::OPENAI_RESPONSES_COMPATIBLE,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAIResponses),
            url: Url::parse("https://provider.example/custom-prefix/v1/responses").unwrap(),
            credential: make_credential(ProviderId::OPENAI_RESPONSES_COMPATIBLE, "test-key"),
            custom_headers: None,
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: None,
        };
        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);

        assert_eq!(
            provider_impl.api_base.as_str(),
            "https://provider.example/custom-prefix/v1"
        );
        assert_eq!(
            provider_impl.responses_url.as_str(),
            "https://provider.example/custom-prefix/v1/responses"
        );
    }

    #[test]
    fn test_openai_responses_provider_new_with_codex_url() {
        let provider = Provider {
            id: ProviderId::CODEX,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://chatgpt.com/backend-api/codex/responses").unwrap(),
            credential: make_credential(ProviderId::CODEX, "test-key"),
            custom_headers: None,
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: None,
        };
        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);

        assert_eq!(
            provider_impl.responses_url.as_str(),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            provider_impl.api_base.as_str(),
            "https://chatgpt.com/backend-api/codex"
        );
    }

    #[test]
    fn test_openai_responses_provider_new_with_oauth_with_api_key() {
        let provider = Provider {
            id: ProviderId::OPENAI,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1").unwrap(),
            credential: Some(forge_domain::AuthCredential {
                id: ProviderId::OPENAI,
                auth_details: forge_domain::AuthDetails::OAuthWithApiKey {
                    tokens: forge_domain::OAuthTokens::new(
                        "access-token",
                        None::<String>,
                        chrono::Utc::now() + chrono::Duration::hours(1),
                    ),
                    api_key: forge_domain::ApiKey::from("oauth-key".to_string()),
                    config: forge_domain::OAuthConfig {
                        auth_url: Url::parse("https://example.com/auth").unwrap(),
                        token_url: Url::parse("https://example.com/token").unwrap(),
                        client_id: forge_domain::ClientId::from("client-id".to_string()),
                        scopes: vec![],
                        redirect_uri: None,
                        use_pkce: false,
                        token_refresh_url: None,
                        custom_headers: None,
                        extra_auth_params: None,
                    },
                },
                url_params: HashMap::new(),
            }),
            auth_methods: vec![],
            url_params: vec![],
            models: None,
            custom_headers: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        assert_eq!(provider_impl.api_base.as_str(), "https://api.openai.com/v1");
    }

    #[test]
    fn test_openai_responses_provider_new_with_oauth() {
        let provider = Provider {
            id: ProviderId::OPENAI,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1").unwrap(),
            credential: Some(forge_domain::AuthCredential {
                id: ProviderId::OPENAI,
                auth_details: forge_domain::AuthDetails::OAuth {
                    tokens: forge_domain::OAuthTokens::new(
                        "access-token",
                        None::<String>,
                        chrono::Utc::now() + chrono::Duration::hours(1),
                    ),
                    config: forge_domain::OAuthConfig {
                        auth_url: Url::parse("https://example.com/auth").unwrap(),
                        token_url: Url::parse("https://example.com/token").unwrap(),
                        client_id: forge_domain::ClientId::from("client-id".to_string()),
                        scopes: vec![],
                        redirect_uri: None,
                        use_pkce: false,
                        token_refresh_url: None,
                        custom_headers: None,
                        extra_auth_params: None,
                    },
                },
                url_params: HashMap::new(),
            }),
            auth_methods: vec![],
            url_params: vec![],
            models: None,
            custom_headers: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        assert_eq!(provider_impl.api_base.as_str(), "https://api.openai.com/v1");
    }

    #[test]
    fn test_openai_responses_provider_new_without_credential() {
        let provider = Provider {
            id: ProviderId::OPENAI,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1").unwrap(),
            credential: None,
            custom_headers: None,
            auth_methods: vec![],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        assert_eq!(provider_impl.api_base.as_str(), "https://api.openai.com/v1");
    }

    #[test]
    fn test_get_headers_with_api_key() {
        let provider = openai_responses("test-key", "https://api.openai.com/v1");
        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);

        let headers = provider_impl.get_headers();

        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "authorization");
        assert_eq!(headers[0].1, "Bearer test-key");
    }

    #[test]
    fn test_get_headers_with_oauth_device_custom_headers() {
        let provider = Provider {
            id: ProviderId::OPENAI,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1").unwrap(),
            credential: make_credential(ProviderId::OPENAI, "test-key"),
            custom_headers: None,
            auth_methods: vec![forge_domain::AuthMethod::OAuthDevice(
                forge_domain::OAuthConfig {
                    auth_url: Url::parse("https://example.com/auth").unwrap(),
                    token_url: Url::parse("https://example.com/token").unwrap(),
                    client_id: forge_domain::ClientId::from("client-id".to_string()),
                    scopes: vec![],
                    redirect_uri: None,
                    use_pkce: false,
                    token_refresh_url: None,
                    custom_headers: Some(
                        [("X-Custom".to_string(), "value".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    extra_auth_params: None,
                },
            )],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        let headers = provider_impl.get_headers();

        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].0, "authorization");
        assert_eq!(headers[1].0, "X-Custom");
        assert_eq!(headers[1].1, "value");
    }

    #[test]
    fn test_get_headers_with_oauth_code_custom_headers() {
        let provider = Provider {
            id: ProviderId::OPENAI,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1").unwrap(),
            credential: make_credential(ProviderId::OPENAI, "test-key"),
            custom_headers: None,
            auth_methods: vec![forge_domain::AuthMethod::OAuthCode(
                forge_domain::OAuthConfig {
                    auth_url: Url::parse("https://example.com/auth").unwrap(),
                    token_url: Url::parse("https://example.com/token").unwrap(),
                    client_id: forge_domain::ClientId::from("client-id".to_string()),
                    scopes: vec![],
                    redirect_uri: None,
                    use_pkce: false,
                    token_refresh_url: None,
                    custom_headers: Some(
                        [("X-Custom".to_string(), "value".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    extra_auth_params: None,
                },
            )],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        let headers = provider_impl.get_headers();

        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].0, "authorization");
        assert_eq!(headers[1].0, "X-Custom");
        assert_eq!(headers[1].1, "value");
    }

    #[test]
    fn test_into_sse_parse_error_marks_transport_errors_retryable() {
        let error = into_sse_parse_error(forge_eventsource_stream::EventStreamError::Transport(
            anyhow::anyhow!("error decoding response body"),
        ));

        assert!(is_retryable(&error));
        assert_eq!(
            error.to_string(),
            "SSE parse error: Transport error: error decoding response body"
        );
    }

    #[test]
    fn test_into_sse_parse_error_keeps_utf8_errors_non_retryable() {
        let error = into_sse_parse_error(
            forge_eventsource_stream::EventStreamError::<anyhow::Error>::Utf8(
                String::from_utf8(vec![0xFF]).unwrap_err(),
            ),
        );

        assert!(!is_retryable(&error));
        assert_eq!(
            error.to_string(),
            "SSE parse error: UTF8 error: invalid utf-8 sequence of 1 bytes from index 0"
        );
    }

    #[test]
    fn test_get_headers_without_credential() {
        let provider = Provider {
            id: ProviderId::OPENAI,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1").unwrap(),
            credential: None,
            custom_headers: None,
            auth_methods: vec![],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        let headers = provider_impl.get_headers();

        assert!(headers.is_empty());
    }

    #[test]
    fn test_get_headers_with_multiple_custom_headers() {
        let provider = Provider {
            id: ProviderId::OPENAI,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1").unwrap(),
            credential: make_credential(ProviderId::OPENAI, "test-key"),
            custom_headers: None,
            auth_methods: vec![forge_domain::AuthMethod::OAuthDevice(
                forge_domain::OAuthConfig {
                    auth_url: Url::parse("https://example.com/auth").unwrap(),
                    token_url: Url::parse("https://example.com/token").unwrap(),
                    client_id: forge_domain::ClientId::from("client-id".to_string()),
                    scopes: vec![],
                    redirect_uri: None,
                    use_pkce: false,
                    token_refresh_url: None,
                    custom_headers: Some(
                        [
                            ("X-Header1".to_string(), "value1".to_string()),
                            ("X-Header2".to_string(), "value2".to_string()),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    extra_auth_params: None,
                },
            )],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        let headers = provider_impl.get_headers();

        assert_eq!(headers.len(), 3);
        let header_names: Vec<&str> = headers.iter().map(|h| h.0.as_str()).collect();
        assert!(header_names.contains(&"authorization"));
        assert!(header_names.contains(&"X-Header1"));
        assert!(header_names.contains(&"X-Header2"));
    }

    #[test]
    fn test_get_headers_with_codex_device_custom_headers() {
        let provider = Provider {
            id: ProviderId::CODEX,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://chatgpt.com/backend-api/codex/responses").unwrap(),
            credential: make_credential(ProviderId::CODEX, "test-token"),
            custom_headers: None,
            auth_methods: vec![forge_domain::AuthMethod::CodexDevice(
                forge_domain::OAuthConfig {
                    auth_url: Url::parse(
                        "https://auth.openai.com/api/accounts/deviceauth/usercode",
                    )
                    .unwrap(),
                    token_url: Url::parse("https://auth.openai.com/oauth/token").unwrap(),
                    client_id: forge_domain::ClientId::from(
                        "app_EMoamEEZ73f0CkXaXp7hrann".to_string(),
                    ),
                    scopes: vec![],
                    redirect_uri: None,
                    use_pkce: false,
                    token_refresh_url: None,
                    custom_headers: Some(
                        [("originator".to_string(), "forge".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    extra_auth_params: None,
                },
            )],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        let actual = provider_impl.get_headers();

        let header_names: Vec<&str> = actual.iter().map(|h| h.0.as_str()).collect();
        assert!(header_names.contains(&"authorization"));
        assert!(header_names.contains(&"originator"));
    }

    #[test]
    fn test_get_headers_codex_includes_chatgpt_account_id() {
        let mut url_params = HashMap::new();
        url_params.insert(
            forge_domain::URLParam::from("chatgpt_account_id".to_string()),
            forge_domain::URLParamValue::from("acct_test_123".to_string()),
        );

        let provider = Provider {
            id: ProviderId::CODEX,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://chatgpt.com/backend-api/codex/responses").unwrap(),
            credential: Some(forge_domain::AuthCredential {
                id: ProviderId::CODEX,
                auth_details: forge_domain::AuthDetails::OAuth {
                    tokens: forge_domain::OAuthTokens::new(
                        "access-token",
                        None::<String>,
                        chrono::Utc::now() + chrono::Duration::hours(1),
                    ),
                    config: forge_domain::OAuthConfig {
                        auth_url: Url::parse(
                            "https://auth.openai.com/api/accounts/deviceauth/usercode",
                        )
                        .unwrap(),
                        token_url: Url::parse("https://auth.openai.com/oauth/token").unwrap(),
                        client_id: forge_domain::ClientId::from("app_test".to_string()),
                        scopes: vec![],
                        redirect_uri: None,
                        use_pkce: false,
                        token_refresh_url: None,
                        custom_headers: None,
                        extra_auth_params: None,
                    },
                },
                url_params,
            }),
            auth_methods: vec![],
            url_params: vec![],
            models: None,
            custom_headers: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        let actual = provider_impl.get_headers();

        let account_header = actual.iter().find(|(k, _)| k == "ChatGPT-Account-Id");
        assert!(account_header.is_some());
        assert_eq!(account_header.unwrap().1, "acct_test_123");
    }

    #[test]
    fn test_get_headers_codex_omits_chatgpt_account_id_when_missing() {
        let provider = Provider {
            id: ProviderId::CODEX,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://chatgpt.com/backend-api/codex/responses").unwrap(),
            credential: make_credential(ProviderId::CODEX, "test-token"),
            custom_headers: None,
            auth_methods: vec![],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        let actual = provider_impl.get_headers();

        let account_header = actual.iter().find(|(k, _)| k == "ChatGPT-Account-Id");
        assert!(account_header.is_none());
    }

    #[test]
    fn test_get_headers_non_codex_does_not_include_chatgpt_account_id() {
        let mut url_params = HashMap::new();
        url_params.insert(
            forge_domain::URLParam::from("chatgpt_account_id".to_string()),
            forge_domain::URLParamValue::from("acct_should_not_appear".to_string()),
        );

        let provider = Provider {
            id: ProviderId::OPENAI,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1").unwrap(),
            credential: Some(forge_domain::AuthCredential {
                id: ProviderId::OPENAI,
                auth_details: forge_domain::AuthDetails::ApiKey(forge_domain::ApiKey::from(
                    "test-key".to_string(),
                )),
                url_params,
            }),
            auth_methods: vec![],
            url_params: vec![],
            models: None,
            custom_headers: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        let actual = provider_impl.get_headers();

        let account_header = actual.iter().find(|(k, _)| k == "ChatGPT-Account-Id");
        assert!(account_header.is_none());
    }

    #[test]
    fn test_get_headers_codex_with_conversation_id_includes_conversation_headers() {
        let provider = Provider {
            id: ProviderId::CODEX,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://chatgpt.com/backend-api/codex/responses").unwrap(),
            credential: make_credential(ProviderId::CODEX, "test-token"),
            custom_headers: None,
            auth_methods: vec![],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);
        let fixture = "conversation_test_123";

        let actual = provider_impl.get_headers_for_conversation(Some(fixture));

        let x_client_request_id = actual
            .iter()
            .find(|(k, _)| k == "x-client-request-id")
            .map(|(_, v)| v.as_str());
        let session_id = actual
            .iter()
            .find(|(k, _)| k == "session_id")
            .map(|(_, v)| v.as_str());

        let expected = Some(fixture);
        assert_eq!(x_client_request_id, expected);
        assert_eq!(session_id, expected);
    }

    #[test]
    fn test_get_headers_non_codex_with_conversation_id_omits_conversation_headers() {
        let provider = openai_responses("test-key", "https://api.openai.com/v1");
        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);

        let actual = provider_impl.get_headers_for_conversation(Some("conversation_test_123"));

        let x_client_request_id = actual.iter().find(|(k, _)| k == "x-client-request-id");
        let session_id = actual.iter().find(|(k, _)| k == "session_id");

        assert!(x_client_request_id.is_none());
        assert!(session_id.is_none());
    }

    #[test]
    fn test_get_headers_codex_without_conversation_id_omits_conversation_headers() {
        let provider = Provider {
            id: ProviderId::CODEX,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://chatgpt.com/backend-api/codex/responses").unwrap(),
            credential: make_credential(ProviderId::CODEX, "test-token"),
            custom_headers: None,
            auth_methods: vec![],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::<MockHttpClient>::new(provider, infra);

        let actual = provider_impl.get_headers_for_conversation(None);

        let x_client_request_id = actual.iter().find(|(k, _)| k == "x-client-request-id");
        let session_id = actual.iter().find(|(k, _)| k == "session_id");

        assert!(x_client_request_id.is_none());
        assert!(session_id.is_none());
    }

    #[tokio::test]
    async fn test_openai_responses_repository_models_returns_empty() -> anyhow::Result<()> {
        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let repo = OpenAIResponsesResponseRepository::new(infra);

        let provider = openai_responses("test-key", "https://api.openai.com/v1");
        let models = repo.models(provider).await?;

        assert!(models.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_openai_responses_provider_uses_direct_http_calls() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;

        // Create SSE events for streaming response
        let events = vec![
            "event: response.output_text.delta".to_string(),
            format!(
                "data: {}",
                serde_json::json!({
                    "type": "response.output_text.delta",
                    "sequence_number": 1,
                    "item_id": "item_1",
                    "output_index": 0,
                    "content_index": 0,
                    "delta": "hello"
                })
            ),
            "event: response.completed".to_string(),
            format!(
                "data: {}",
                serde_json::json!({
                    "type": "response.completed",
                    "sequence_number": 2,
                    "response": openai_response_fixture()
                })
            ),
            "event: done".to_string(),
            "data: [DONE]".to_string(),
        ];

        let mock = fixture.mock_responses_stream(events, 200).await;

        let provider = openai_responses(
            "test-api-key",
            &format!("{}/v1/chat/completions", fixture.url()),
        );

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl: OpenAIResponsesProvider<_> =
            OpenAIResponsesProvider::new(provider, infra);
        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hi", None))
            .stream(true);

        let mut stream = provider_impl
            .chat(&ModelId::from("codex-mini-latest"), context)
            .await?;

        let first = stream.next().await.expect("stream should yield")?;

        mock.assert_async().await;
        assert_eq!(first.content, Some(Content::part("hello")));

        let second = stream
            .next()
            .await
            .expect("stream should yield second message")?;
        assert_eq!(second.finish_reason, Some(FinishReason::Stop));

        Ok(())
    }

    /// Tests the Codex direct streaming path (`chat_codex_stream`) which
    /// bypasses the Content-Type validation enforced by reqwest-eventsource.
    /// The mock server returns SSE data with `Content-Type:
    /// application/octet-stream` (not `text/event-stream`), verifying the
    /// bypass works correctly.
    #[tokio::test]
    async fn test_codex_provider_streams_without_text_event_stream_content_type()
    -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;

        let events = vec![
            "event: response.output_text.delta".to_string(),
            format!(
                "data: {}",
                serde_json::json!({
                    "type": "response.output_text.delta",
                    "sequence_number": 1,
                    "item_id": "item_1",
                    "output_index": 0,
                    "content_index": 0,
                    "delta": "hello from codex"
                })
            ),
            "event: response.completed".to_string(),
            format!(
                "data: {}",
                serde_json::json!({
                    "type": "response.completed",
                    "sequence_number": 2,
                    "response": openai_response_fixture()
                })
            ),
            "event: done".to_string(),
            "data: [DONE]".to_string(),
        ];

        let mock = fixture
            .mock_codex_responses_stream("/backend-api/codex/responses", events, 200)
            .await;

        let codex_url = format!("{}/backend-api/codex/responses", fixture.url());
        let provider = Provider {
            id: ProviderId::CODEX,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse(&codex_url).unwrap(),
            credential: make_credential(ProviderId::CODEX, "test-codex-token"),
            custom_headers: None,
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::new(provider, infra);
        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hi", None))
            .stream(true);

        let mut stream = provider_impl
            .chat(&ModelId::from("gpt-5.1-codex-mini"), context)
            .await?;

        let first = stream.next().await.expect("stream should yield")?;
        mock.assert_async().await;
        assert_eq!(first.content, Some(Content::part("hello from codex")));

        let second = stream
            .next()
            .await
            .expect("stream should yield second message")?;
        assert_eq!(second.finish_reason, Some(FinishReason::Stop));

        Ok(())
    }

    /// Tests that the Codex stream silently skips keepalive events that
    /// cannot be deserialized as `ResponseStreamEvent`.
    #[tokio::test]
    async fn test_codex_provider_skips_keepalive_events() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;

        let events = vec![
            "event: response.output_text.delta".to_string(),
            format!(
                "data: {}",
                serde_json::json!({
                    "type": "response.output_text.delta",
                    "sequence_number": 1,
                    "item_id": "item_1",
                    "output_index": 0,
                    "content_index": 0,
                    "delta": "hello"
                })
            ),
            // Keepalive event that should be silently skipped
            "event: keepalive".to_string(),
            format!(
                "data: {}",
                serde_json::json!({
                    "type": "keepalive",
                    "sequence_number": 2
                })
            ),
            "event: response.completed".to_string(),
            format!(
                "data: {}",
                serde_json::json!({
                    "type": "response.completed",
                    "sequence_number": 3,
                    "response": openai_response_fixture()
                })
            ),
            "event: done".to_string(),
            "data: [DONE]".to_string(),
        ];

        let mock = fixture
            .mock_codex_responses_stream("/backend-api/codex/responses", events, 200)
            .await;

        let codex_url = format!("{}/backend-api/codex/responses", fixture.url());
        let provider = Provider {
            id: ProviderId::CODEX,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse(&codex_url).unwrap(),
            credential: make_credential(ProviderId::CODEX, "test-codex-token"),
            custom_headers: None,
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::new(provider, infra);
        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hi", None))
            .stream(true);

        let mut stream = provider_impl
            .chat(&ModelId::from("gpt-5.1-codex-mini"), context)
            .await?;

        // First message should be the text delta (keepalive was skipped)
        let first = stream.next().await.expect("stream should yield")?;
        mock.assert_async().await;
        assert_eq!(first.content, Some(Content::part("hello")));

        // Second message should be the completion event
        let second = stream
            .next()
            .await
            .expect("stream should yield second message")?;
        assert_eq!(second.finish_reason, Some(FinishReason::Stop));

        Ok(())
    }

    /// Tests that the Codex stream correctly returns an error for non-success
    /// HTTP status codes.
    #[tokio::test]
    async fn test_codex_provider_stream_returns_error_on_non_success() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;

        let _mock = fixture
            .mock_codex_responses_stream("/backend-api/codex/responses", vec![], 400)
            .await;

        let codex_url = format!("{}/backend-api/codex/responses", fixture.url());
        let provider = Provider {
            id: ProviderId::CODEX,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse(&codex_url).unwrap(),
            credential: make_credential(ProviderId::CODEX, "test-codex-token"),
            custom_headers: None,
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: None,
        };

        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::new(provider, infra);
        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hi", None))
            .stream(true);

        let actual = provider_impl
            .chat(&ModelId::from("gpt-5.1-codex"), context)
            .await;

        assert!(actual.is_err());

        Ok(())
    }

    /// Tests that when the SSE endpoint returns a non-2xx status the stream
    /// error includes both the response body and the URL.
    #[tokio::test]
    async fn test_stream_error_on_non_success_includes_body_and_url() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;
        let error_body = r#"{"error":{"message":"The requested model is not supported.","code":"model_not_supported"}}"#;
        let _mock = fixture
            .mock_post_error("/v1/responses", error_body, 400)
            .await;

        let provider = openai_responses(
            "test-api-key",
            &format!("{}/v1/chat/completions", fixture.url()),
        );
        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::new(provider, infra);
        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hi", None))
            .stream(true);

        let mut stream = provider_impl
            .chat(&ModelId::from("gpt-4o"), context)
            .await?;

        let actual = stream.next().await.expect("stream should yield one item");
        assert!(actual.is_err());
        let err_str = format!("{:#}", actual.unwrap_err());
        assert!(
            err_str.contains("400 Bad Request Reason:"),
            "missing reason: {err_str}"
        );
        assert!(
            err_str.contains("model_not_supported"),
            "missing body: {err_str}"
        );
        assert!(err_str.contains("/v1/responses"), "missing url: {err_str}");
        Ok(())
    }

    /// Tests that when the SSE endpoint returns 200 with a non-SSE content type
    /// the stream error includes the response body and the URL.
    #[tokio::test]
    async fn test_stream_error_on_wrong_content_type_includes_body_and_url() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;
        let error_body = r#"{"error":{"message":"internal server error"}}"#;
        let _mock = fixture
            .mock_post_wrong_content_type("/v1/responses", error_body)
            .await;

        let provider = openai_responses(
            "test-api-key",
            &format!("{}/v1/chat/completions", fixture.url()),
        );
        let infra = Arc::new(MockHttpClient { client: reqwest::Client::new() });
        let provider_impl = OpenAIResponsesProvider::new(provider, infra);
        let context = ChatContext::default()
            .add_message(ContextMessage::user("Hi", None))
            .stream(true);

        let mut stream = provider_impl
            .chat(&ModelId::from("gpt-4o"), context)
            .await?;

        let actual = stream.next().await.expect("stream should yield one item");
        assert!(actual.is_err());
        let err_str = format!("{:#}", actual.unwrap_err());
        assert!(
            err_str.contains("200 OK Reason:"),
            "missing reason: {err_str}"
        );
        assert!(
            err_str.contains("internal server error"),
            "missing body: {err_str}"
        );
        assert!(err_str.contains("/v1/responses"), "missing url: {err_str}");
        Ok(())
    }
}
