use std::sync::Arc;

use anyhow::Context as _;
use forge_app::domain::{
    ChatCompletionMessage, Context, Model, ModelId, ResultStream, Transformer,
};
use forge_app::dto::anthropic::{
    AuthSystemMessage, CapitalizeToolNames, DropInvalidToolUse, EnforceStrictObjectSchema,
    EventData, ListModelResponse, McpToolNames, ReasoningTransform, RemoveOutputFormat, Request,
    SanitizeToolIds, SetCache,
};
use forge_app::{EnvironmentInfra, HttpInfra};
use forge_domain::{ChatRepository, Provider, ProviderId};
use forge_eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Url;
use reqwest::header::HeaderMap;
use tracing::debug;

use crate::provider::event::into_chat_completion_message;
use crate::provider::retry::into_retry;
use crate::provider::utils::{create_headers, format_http_context};

#[derive(Clone)]
struct Anthropic<T> {
    http: Arc<T>,
    provider: Provider<Url>,
    anthropic_version: String,
    use_oauth: bool,
}

impl<H: HttpInfra> Anthropic<H> {
    pub fn new(http: Arc<H>, provider: Provider<Url>, version: String, use_oauth: bool) -> Self {
        Self { http, provider, anthropic_version: version, use_oauth }
    }

    fn get_headers(&self, model: Option<&ModelId>) -> Vec<(String, String)> {
        let mut headers = vec![(
            "anthropic-version".to_string(),
            self.anthropic_version.clone(),
        )];

        // Extract API key/token from provider credentials (handles Google ADC, OAuth,
        // and API key)
        let api_key = self
            .provider
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
                forge_domain::AuthDetails::GoogleAdc(api_key) => Some(api_key.as_str()),
                forge_domain::AuthDetails::AwsProfile(_) => None,
            });

        if let Some(api_key) = api_key {
            // For Vertex AI, use Authorization: Bearer with Google ADC token
            // For OAuth, use Authorization: Bearer
            // For API key, use x-api-key header
            if self.provider.id == ProviderId::VERTEX_AI_ANTHROPIC || self.use_oauth {
                headers.push(("authorization".to_string(), format!("Bearer {}", api_key)));
            } else {
                headers.push(("x-api-key".to_string(), api_key.to_string()));
            }
        }

        // Add beta flags (not needed for Vertex AI)
        if self.provider.id != ProviderId::VERTEX_AI_ANTHROPIC {
            let mut betas: Vec<&'static str> = Vec::new();
            if self.use_oauth {
                betas.push("claude-code-20250219");
                betas.push("oauth-2025-04-20");
            }
            // Adaptive thinking auto-enables interleaved thinking on Opus 4.7,
            // Opus 4.6, and Sonnet 4.6 — the beta header is redundant there per
            // the Opus 4.7 migration guide. Keep it for older models so manual
            // `extended-thinking` requests still get interleaved turns.
            if interleaved_thinking_required(model) {
                betas.push("interleaved-thinking-2025-05-14");
            }
            betas.push("structured-outputs-2025-11-13");
            headers.push(("anthropic-beta".to_string(), betas.join(",")));
        }

        headers
    }
}

/// Returns false when the model auto-enables interleaved thinking through
/// adaptive thinking (Opus 4.7, Opus 4.6, Sonnet 4.6). When the model is
/// unknown (e.g., listing endpoints), the flag is included because it is
/// harmless on non-chat endpoints and necessary on older chat models.
fn interleaved_thinking_required(model: Option<&ModelId>) -> bool {
    let Some(model) = model else { return true };
    let id = model.as_str().to_lowercase();
    !(id.contains("opus-4-7") || id.contains("opus-4-6") || id.contains("sonnet-4-6"))
}

impl<T: HttpInfra> Anthropic<T> {
    /// Determines whether this provider should bypass reqwest-eventsource
    /// content-type validation and parse SSE from raw bytes instead.
    fn should_use_raw_sse(&self) -> bool {
        self.provider.id == ProviderId::OPENCODE_ZEN
    }

    pub async fn chat(
        &self,
        model: &ModelId,
        context: Context,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let max_tokens = context.max_tokens.unwrap_or(4000);
        // transform the context to match the request format
        let context = ReasoningTransform.transform(context);

        let mut request = Request::try_from(context)?.max_tokens(max_tokens as u64);

        // For Vertex AI Anthropic, model is in the URL path, not the request body
        if self.provider.id == ProviderId::VERTEX_AI_ANTHROPIC {
            request = request.anthropic_version(self.anthropic_version.clone());
        } else {
            request = request.model(model.as_str().to_string());
        }

        let pipeline = AuthSystemMessage::default()
            .when(|_| self.use_oauth)
            .pipe(McpToolNames.when(|_| self.use_oauth))
            .pipe(CapitalizeToolNames)
            .pipe(DropInvalidToolUse)
            .pipe(SanitizeToolIds);

        // Vertex AI does not support output_format, so we skip schema enforcement
        // and remove any output_format field
        let request = if self.provider.id == ProviderId::VERTEX_AI_ANTHROPIC {
            pipeline
                .pipe(RemoveOutputFormat)
                .pipe(SetCache)
                .transform(request)
        } else {
            pipeline
                .pipe(EnforceStrictObjectSchema)
                .pipe(SetCache)
                .transform(request)
        };

        let url = if self.provider.id == ProviderId::VERTEX_AI_ANTHROPIC {
            // For Vertex AI, we need to append the model ID and streamRawPredict to the URL
            // The chat_url from provider.json ends with .../models
            let base = self.provider.url.as_str().trim_end_matches('/');
            format!("{}/{}:streamRawPredict", base, model.as_str())
        } else {
            self.provider.url.to_string()
        };

        debug!(url = %url, model = %model, "Connecting Upstream");

        let json_bytes =
            serde_json::to_vec(&request).with_context(|| "Failed to serialize request")?;

        let parsed_url = Url::parse(&url).with_context(|| format!("Invalid URL: {}", url))?;
        let headers = create_headers(self.get_headers(Some(model)));

        if self.should_use_raw_sse() {
            return self.chat_raw_sse(&parsed_url, headers, json_bytes).await;
        }

        let source = self
            .http
            .http_eventsource(&parsed_url, Some(headers), json_bytes.into())
            .await
            .with_context(|| format_http_context(None, "POST", &url))?;

        let stream = into_chat_completion_message::<EventData>(parsed_url, source);

        Ok(Box::pin(stream))
    }

    /// Streams Anthropic events from a raw byte response body and parses
    /// SSE events manually. This bypasses reqwest-eventsource content-type
    /// validation for providers that return non-standard SSE content types.
    async fn chat_raw_sse(
        &self,
        parsed_url: &Url,
        headers: HeaderMap,
        json_bytes: Vec<u8>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let response = self
            .http
            .http_post(parsed_url, Some(headers), json_bytes.into())
            .await
            .with_context(|| format_http_context(None, "POST", parsed_url))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read response body".to_string());
            return Err(anyhow::anyhow!(error_body))
                .with_context(|| format_http_context(Some(status), "POST", parsed_url));
        }

        let request_url = parsed_url.clone();
        let stream = response
            .bytes_stream()
            .eventsource()
            .filter_map(move |event_result| {
                let request_url = request_url.clone();
                async move {
                    match event_result {
                        Ok(event) if ["[DONE]", ""].contains(&event.data.as_str()) => None,
                        Ok(event) => Some(
                            serde_json::from_str::<EventData>(&event.data)
                                .with_context(|| {
                                    format!("Failed to parse provider response: {}", event.data)
                                })
                                .and_then(|response| {
                                    ChatCompletionMessage::try_from(response).with_context(|| {
                                        format!(
                                            "Failed to create completion message: {}",
                                            event.data
                                        )
                                    })
                                })
                                .with_context(|| {
                                    format_http_context(None, "POST", request_url.clone())
                                }),
                        ),
                        Err(error) => Some(Err(into_sse_parse_error(error)).with_context(|| {
                            format_http_context(None, "POST", request_url.clone())
                        })),
                    }
                }
            });

        Ok(Box::pin(stream))
    }

    pub async fn models(&self) -> anyhow::Result<Vec<Model>> {
        let models = self
            .provider
            .models
            .as_ref()
            .context("Anthropic requires models configuration")?;

        match models {
            forge_domain::ModelSource::Url(url) => {
                debug!(url = %url, "Fetching models");

                let response = self
                    .http
                    .http_get(url, Some(create_headers(self.get_headers(None))))
                    .await
                    .with_context(|| format_http_context(None, "GET", url))
                    .with_context(|| "Failed to fetch models")?;

                let status = response.status();
                let ctx_msg = format_http_context(Some(status), "GET", url);
                let text = response
                    .text()
                    .await
                    .with_context(|| ctx_msg.clone())
                    .with_context(|| "Failed to decode response into text")?;

                if status.is_success() {
                    let response: ListModelResponse = serde_json::from_str(&text)
                        .with_context(|| ctx_msg)
                        .with_context(|| "Failed to deserialize models response")?;
                    Ok(response.data.into_iter().map(Into::into).collect())
                } else {
                    // treat non 200 response as error.
                    Err(anyhow::anyhow!(text))
                        .with_context(|| ctx_msg)
                        .with_context(|| "Failed to fetch the models")
                }
            }
            forge_domain::ModelSource::Hardcoded(models) => {
                debug!("Using hardcoded models");
                Ok(models.clone())
            }
        }
    }
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

/// Repository for Anthropic provider responses
pub struct AnthropicResponseRepository<F> {
    infra: Arc<F>,
}

impl<F> AnthropicResponseRepository<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

impl<F: HttpInfra> AnthropicResponseRepository<F> {
    /// Creates an Anthropic client from a provider configuration
    fn create_client(&self, provider: Provider<Url>) -> anyhow::Result<Anthropic<F>> {
        // Validate that credentials exist
        provider
            .credential
            .as_ref()
            .context("Anthropic provider requires credentials")?;

        // Determine OAuth usage based on auth details
        let is_oauth = provider
            .credential
            .as_ref()
            .map(|c| matches!(c.auth_details, forge_domain::AuthDetails::OAuth { .. }))
            .unwrap_or(false);

        // Use different API version for Vertex AI
        let version = if provider.id == ProviderId::VERTEX_AI_ANTHROPIC {
            "vertex-2023-10-16".to_string()
        } else {
            "2023-06-01".to_string()
        };

        Ok(Anthropic::new(
            self.infra.clone(),
            provider,
            version,
            is_oauth,
        ))
    }
}

#[async_trait::async_trait]
impl<F: HttpInfra + EnvironmentInfra<Config = forge_config::ForgeConfig> + 'static> ChatRepository
    for AnthropicResponseRepository<F>
{
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let retry_config = self.infra.get_config()?.retry.unwrap_or_default();
        let provider_client = self.create_client(provider)?;

        let stream = provider_client
            .chat(model_id, context)
            .await
            .map_err(|e| into_retry(e, &retry_config))?;

        Ok(Box::pin(stream.map(move |item| {
            item.map_err(|e| into_retry(e, &retry_config))
        })))
    }

    async fn models(&self, provider: Provider<Url>) -> anyhow::Result<Vec<Model>> {
        let retry_config = self.infra.get_config()?.retry.unwrap_or_default();
        let provider_client = self.create_client(provider)?;

        provider_client
            .models()
            .await
            .map_err(|e| into_retry(e, &retry_config))
            .context("Failed to fetch models from Anthropic provider")
    }
}

#[cfg(test)]
mod tests {

    use bytes::Bytes;
    use forge_app::HttpInfra;
    use forge_app::domain::{
        Context, ContextMessage, ToolCallFull, ToolCallId, ToolChoice, ToolName, ToolOutput,
        ToolResult,
    };
    use forge_eventsource::EventSource;
    use reqwest::header::HeaderMap;

    use super::*;
    use crate::provider::mock_server::{MockServer, normalize_ports};

    // Mock implementation of HttpInfra for testing
    #[derive(Clone)]
    struct MockHttpClient {
        client: reqwest::Client,
    }

    impl MockHttpClient {
        fn new() -> Self {
            Self { client: reqwest::Client::new() }
        }
    }

    #[async_trait::async_trait]
    impl HttpInfra for MockHttpClient {
        async fn http_get(
            &self,
            url: &Url,
            headers: Option<HeaderMap>,
        ) -> anyhow::Result<reqwest::Response> {
            let mut request = self.client.get(url.clone());
            if let Some(headers) = headers {
                request = request.headers(headers);
            }
            Ok(request.send().await?)
        }

        async fn http_post(
            &self,
            _url: &Url,
            _headers: Option<HeaderMap>,
            _body: Bytes,
        ) -> anyhow::Result<reqwest::Response> {
            unimplemented!()
        }

        async fn http_delete(&self, _url: &Url) -> anyhow::Result<reqwest::Response> {
            unimplemented!()
        }

        async fn http_eventsource(
            &self,
            _url: &Url,
            _headers: Option<HeaderMap>,
            _body: Bytes,
        ) -> anyhow::Result<EventSource> {
            // For now, return an error since eventsource is not used in the failing tests
            Err(anyhow::anyhow!("EventSource not implemented in mock"))
        }
    }

    fn create_anthropic(base_url: &str) -> anyhow::Result<Anthropic<MockHttpClient>> {
        let chat_url = Url::parse(base_url)?.join("messages")?;
        let model_url = Url::parse(base_url)?.join("models")?;

        let provider = Provider {
            id: forge_app::domain::ProviderId::ANTHROPIC,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(forge_app::domain::ProviderResponse::Anthropic),
            url: chat_url,
            credential: Some(forge_domain::AuthCredential {
                id: forge_app::domain::ProviderId::ANTHROPIC,
                auth_details: forge_domain::AuthDetails::ApiKey(forge_domain::ApiKey::from(
                    "sk-test-key".to_string(),
                )),
                url_params: std::collections::HashMap::new(),
            }),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: Some(forge_domain::ModelSource::Url(model_url)),
            custom_headers: None,
        };

        Ok(Anthropic::new(
            Arc::new(MockHttpClient::new()),
            provider,
            "2023-06-01".to_string(),
            false,
        ))
    }

    fn create_mock_models_response() -> serde_json::Value {
        serde_json::json!({
            "data": [
                {
                    "type": "model",
                    "id": "claude-3-5-sonnet-20241022",
                    "display_name": "Claude 3.5 Sonnet (New)",
                    "created_at": "2024-10-22T00:00:00Z"
                },
                {
                    "type": "model",
                    "id": "claude-3-5-haiku-20241022",
                    "display_name": "Claude 3.5 Haiku",
                    "created_at": "2024-10-22T00:00:00Z"
                }
            ],
            "has_more": false,
            "first_id": "claude-3-5-sonnet-20241022",
            "last_id": "claude-3-opus-20240229"
        })
    }

    fn create_error_response(message: &str, code: u16) -> serde_json::Value {
        serde_json::json!({
            "error": {
                "code": code,
                "message": message
            }
        })
    }

    fn create_empty_response() -> serde_json::Value {
        serde_json::json!({
            "data": [],
        })
    }

    #[tokio::test]
    async fn test_url_for_models() {
        let chat_url = Url::parse("https://api.anthropic.com/v1/messages").unwrap();
        let model_url = Url::parse("https://api.anthropic.com/v1/models").unwrap();

        let provider = Provider {
            id: forge_app::domain::ProviderId::ANTHROPIC,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(forge_app::domain::ProviderResponse::Anthropic),
            url: chat_url,
            credential: Some(forge_domain::AuthCredential {
                id: forge_app::domain::ProviderId::ANTHROPIC,
                auth_details: forge_domain::AuthDetails::ApiKey(forge_domain::ApiKey::from(
                    "sk-some-key".to_string(),
                )),
                url_params: std::collections::HashMap::new(),
            }),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: Some(forge_domain::ModelSource::Url(model_url.clone())),
            custom_headers: None,
        };

        let anthropic = Anthropic::new(
            Arc::new(MockHttpClient::new()),
            provider.clone(),
            "v1".to_string(),
            false,
        );
        match &anthropic.provider.models {
            Some(forge_domain::ModelSource::Url(url)) => {
                assert_eq!(url.as_str(), "https://api.anthropic.com/v1/models");
            }
            _ => panic!("Expected Models::Url variant"),
        }
    }

    #[tokio::test]
    async fn test_request_conversion() {
        let model_id = ModelId::new("gpt-4");
        let context = Context::default()
            .add_message(ContextMessage::system(
                "You're expert at math, so you should resolve all user queries.",
            ))
            .add_message(ContextMessage::user(
                "what's 2 + 2 ?",
                model_id.clone().into(),
            ))
            .add_message(ContextMessage::assistant(
                "here is the system call.",
                None,
                None,
                Some(vec![ToolCallFull {
                    name: ToolName::new("math"),
                    call_id: Some(ToolCallId::new("math-1")),
                    arguments: serde_json::json!({"expression": "2 + 2"}).into(),
                    thought_signature: None,
                }]),
            ))
            .add_tool_results(vec![ToolResult {
                name: ToolName::new("math"),
                call_id: Some(ToolCallId::new("math-1")),
                output: ToolOutput::text(serde_json::json!({"result": 4}).to_string()),
            }])
            .tool_choice(ToolChoice::Call(ToolName::new("math")));
        let request = Request::try_from(context)
            .unwrap()
            .model("sonnet-3.5".to_string())
            .stream(true)
            .max_tokens(4000u64);
        insta::assert_snapshot!(serde_json::to_string_pretty(&request).unwrap());
    }

    #[tokio::test]
    async fn test_fetch_models_success() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;
        let mock = fixture
            .mock_models(create_mock_models_response(), 200)
            .await;
        let anthropic = create_anthropic(&fixture.url())?;
        let actual = anthropic.models().await?;

        mock.assert_async().await;

        // Verify we got the expected models
        assert_eq!(actual.len(), 2);
        insta::assert_json_snapshot!(actual);
        Ok(())
    }

    #[tokio::test]
    async fn test_fetch_models_http_error_status() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;
        let mock = fixture
            .mock_models(create_error_response("Invalid API key", 401), 401)
            .await;

        let anthropic = create_anthropic(&fixture.url())?;
        let actual = anthropic.models().await;

        mock.assert_async().await;

        // Verify that we got an error
        assert!(actual.is_err());
        insta::assert_snapshot!(normalize_ports(format!("{:#?}", actual.unwrap_err())));
        Ok(())
    }

    #[tokio::test]
    async fn test_fetch_models_server_error() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;
        let mock = fixture
            .mock_models(create_error_response("Internal Server Error", 500), 500)
            .await;

        let anthropic = create_anthropic(&fixture.url())?;
        let actual = anthropic.models().await;

        mock.assert_async().await;

        // Verify that we got an error
        assert!(actual.is_err());
        insta::assert_snapshot!(normalize_ports(format!("{:#?}", actual.unwrap_err())));

        Ok(())
    }

    #[tokio::test]
    async fn test_fetch_models_empty_response() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;
        let mock = fixture.mock_models(create_empty_response(), 200).await;

        let anthropic = create_anthropic(&fixture.url())?;
        let actual = anthropic.models().await?;

        mock.assert_async().await;
        assert!(actual.is_empty());
        Ok(())
    }

    #[test]
    fn test_get_headers_with_api_key_includes_beta_flags() {
        let chat_url = Url::parse("https://api.anthropic.com/v1/messages").unwrap();
        let model_url = Url::parse("https://api.anthropic.com/v1/models").unwrap();

        let provider = Provider {
            id: forge_app::domain::ProviderId::ANTHROPIC,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(forge_app::domain::ProviderResponse::Anthropic),
            url: chat_url,
            credential: Some(forge_domain::AuthCredential {
                id: forge_app::domain::ProviderId::ANTHROPIC,
                auth_details: forge_domain::AuthDetails::ApiKey(forge_domain::ApiKey::from(
                    "sk-test-key".to_string(),
                )),
                url_params: std::collections::HashMap::new(),
            }),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: Some(forge_domain::ModelSource::Url(model_url)),
            custom_headers: None,
        };

        let fixture = Anthropic::new(
            Arc::new(MockHttpClient::new()),
            provider,
            "2023-06-01".to_string(),
            false, // API key auth (not OAuth)
        );

        let actual = fixture.get_headers(None);

        // Should contain anthropic-version header
        assert!(
            actual
                .iter()
                .any(|(k, v)| k == "anthropic-version" && v == "2023-06-01")
        );

        // Should contain x-api-key header (not authorization)
        assert!(
            actual
                .iter()
                .any(|(k, v)| k == "x-api-key" && v == "sk-test-key")
        );

        // Should contain anthropic-beta header with structured outputs support
        let beta_header = actual.iter().find(|(k, _)| k == "anthropic-beta");
        assert!(
            beta_header.is_some(),
            "anthropic-beta header should be present for API key auth"
        );

        let (_, beta_value) = beta_header.unwrap();
        assert!(
            beta_value.contains("structured-outputs-2025-11-13"),
            "Beta header should include structured-outputs flag"
        );
        // When the model is unknown (e.g., model listing), keep the
        // interleaved-thinking header since it is harmless on non-chat
        // endpoints and still required for older chat models.
        assert!(
            beta_value.contains("interleaved-thinking-2025-05-14"),
            "Beta header should include interleaved-thinking flag when model is unknown"
        );
    }

    #[test]
    fn test_get_headers_with_oauth_includes_beta_flags() {
        let chat_url = Url::parse("https://api.anthropic.com/v1/messages").unwrap();
        let model_url = Url::parse("https://api.anthropic.com/v1/models").unwrap();

        let provider = Provider {
            id: forge_app::domain::ProviderId::ANTHROPIC,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(forge_app::domain::ProviderResponse::Anthropic),
            url: chat_url,
            credential: Some(forge_domain::AuthCredential {
                id: forge_app::domain::ProviderId::ANTHROPIC,
                auth_details: forge_domain::AuthDetails::OAuth {
                    tokens: forge_domain::OAuthTokens::new(
                        "oauth-token",
                        None::<String>,
                        chrono::Utc::now() + chrono::Duration::hours(1),
                    ),
                    config: forge_domain::OAuthConfig {
                        auth_url: reqwest::Url::parse("https://example.com/auth").unwrap(),
                        token_url: reqwest::Url::parse("https://example.com/token").unwrap(),
                        client_id: forge_domain::ClientId::from("client-id".to_string()),
                        scopes: vec![],
                        redirect_uri: None,
                        use_pkce: false,
                        token_refresh_url: None,
                        custom_headers: None,
                        extra_auth_params: None,
                    },
                },
                url_params: std::collections::HashMap::new(),
            }),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: Some(forge_domain::ModelSource::Url(model_url)),
            custom_headers: None,
        };

        let fixture = Anthropic::new(
            Arc::new(MockHttpClient::new()),
            provider,
            "2023-06-01".to_string(),
            true, // OAuth auth
        );

        let actual = fixture.get_headers(None);

        // Should contain anthropic-version header
        assert!(
            actual
                .iter()
                .any(|(k, v)| k == "anthropic-version" && v == "2023-06-01")
        );

        // Should contain authorization header (not x-api-key)
        assert!(
            actual
                .iter()
                .any(|(k, v)| k == "authorization" && v == "Bearer oauth-token")
        );

        // Should contain anthropic-beta header with structured outputs support
        let beta_header = actual.iter().find(|(k, _)| k == "anthropic-beta");
        assert!(
            beta_header.is_some(),
            "anthropic-beta header should be present for OAuth"
        );

        let (_, beta_value) = beta_header.unwrap();
        assert!(
            beta_value.contains("structured-outputs-2025-11-13"),
            "Beta header should include structured-outputs flag"
        );
        assert!(
            beta_value.contains("oauth-2025-04-20"),
            "Beta header should include oauth flag for OAuth auth"
        );
    }

    #[test]
    fn test_get_headers_drops_interleaved_thinking_for_4_6_plus_models() {
        // Adaptive thinking auto-enables interleaved thinking on Opus 4.7,
        // Opus 4.6, and Sonnet 4.6; the beta header is redundant there.
        let chat_url = Url::parse("https://api.anthropic.com/v1/messages").unwrap();
        let model_url = Url::parse("https://api.anthropic.com/v1/models").unwrap();

        let provider = Provider {
            id: forge_app::domain::ProviderId::ANTHROPIC,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(forge_app::domain::ProviderResponse::Anthropic),
            url: chat_url,
            credential: Some(forge_domain::AuthCredential {
                id: forge_app::domain::ProviderId::ANTHROPIC,
                auth_details: forge_domain::AuthDetails::ApiKey(forge_domain::ApiKey::from(
                    "sk-test-key".to_string(),
                )),
                url_params: std::collections::HashMap::new(),
            }),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: Some(forge_domain::ModelSource::Url(model_url)),
            custom_headers: None,
        };

        let fixture = Anthropic::new(
            Arc::new(MockHttpClient::new()),
            provider,
            "2023-06-01".to_string(),
            false,
        );

        for model_id in [
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-sonnet-4-6",
            "us.anthropic.claude-opus-4-7",
            "global.anthropic.claude-sonnet-4-6",
        ] {
            let model = ModelId::new(model_id);
            let actual = fixture.get_headers(Some(&model));
            let (_, beta_value) = actual
                .iter()
                .find(|(k, _)| k == "anthropic-beta")
                .expect("anthropic-beta header should be present");
            assert!(
                !beta_value.contains("interleaved-thinking-2025-05-14"),
                "Beta header should NOT include interleaved-thinking flag for {} (auto-enabled by adaptive thinking)",
                model_id
            );
            assert!(
                beta_value.contains("structured-outputs-2025-11-13"),
                "structured-outputs flag must still be present for {}",
                model_id
            );
        }
    }

    #[test]
    fn test_get_headers_keeps_interleaved_thinking_for_pre_4_6_models() {
        let chat_url = Url::parse("https://api.anthropic.com/v1/messages").unwrap();
        let model_url = Url::parse("https://api.anthropic.com/v1/models").unwrap();

        let provider = Provider {
            id: forge_app::domain::ProviderId::ANTHROPIC,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(forge_app::domain::ProviderResponse::Anthropic),
            url: chat_url,
            credential: Some(forge_domain::AuthCredential {
                id: forge_app::domain::ProviderId::ANTHROPIC,
                auth_details: forge_domain::AuthDetails::ApiKey(forge_domain::ApiKey::from(
                    "sk-test-key".to_string(),
                )),
                url_params: std::collections::HashMap::new(),
            }),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: Some(forge_domain::ModelSource::Url(model_url)),
            custom_headers: None,
        };

        let fixture = Anthropic::new(
            Arc::new(MockHttpClient::new()),
            provider,
            "2023-06-01".to_string(),
            false,
        );

        for model_id in [
            "claude-opus-4-5-20251101",
            "claude-sonnet-4-5-20250929",
            "claude-haiku-4-5-20251001",
            "claude-opus-4-1-20250805",
            "claude-3-7-sonnet-20250219",
        ] {
            let model = ModelId::new(model_id);
            let actual = fixture.get_headers(Some(&model));
            let (_, beta_value) = actual
                .iter()
                .find(|(k, _)| k == "anthropic-beta")
                .expect("anthropic-beta header should be present");
            assert!(
                beta_value.contains("interleaved-thinking-2025-05-14"),
                "Beta header should include interleaved-thinking flag for pre-4.6 model {}",
                model_id
            );
        }
    }

    #[test]
    fn test_vertex_ai_removes_output_format() {
        use forge_domain::ResponseFormat;
        use schemars::JsonSchema;
        use serde::Deserialize;

        #[derive(Deserialize, JsonSchema)]
        #[schemars(title = "test_response")]
        #[allow(dead_code)]
        struct TestResponse {
            result: String,
        }

        let chat_url = Url::parse(
            "https://aiplatform.googleapis.com/v1/projects/test/locations/global/publishers/anthropic/models",
        )
        .unwrap();

        let provider = Provider {
            id: ProviderId::VERTEX_AI_ANTHROPIC,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(forge_app::domain::ProviderResponse::Anthropic),
            url: chat_url,
            credential: Some(forge_domain::AuthCredential {
                id: ProviderId::VERTEX_AI_ANTHROPIC,
                auth_details: forge_domain::AuthDetails::GoogleAdc(forge_domain::ApiKey::from(
                    "test-token".to_string(),
                )),
                url_params: std::collections::HashMap::new(),
            }),
            auth_methods: vec![forge_domain::AuthMethod::GoogleAdc],
            url_params: vec![],
            models: Some(forge_domain::ModelSource::Hardcoded(vec![])),
            custom_headers: None,
        };

        let _anthropic = Anthropic::new(
            Arc::new(MockHttpClient::new()),
            provider,
            "vertex-2023-10-16".to_string(),
            false,
        );

        // Create a context with response_format (which would normally add
        // output_format)
        let schema = schemars::schema_for!(TestResponse);
        let context = Context::default()
            .add_message(ContextMessage::user("test", ModelId::new("test").into()))
            .response_format(ResponseFormat::JsonSchema(Box::new(schema)));

        // Convert to request
        let mut request = Request::try_from(context).unwrap().max_tokens(4000u64);
        request = request.anthropic_version("vertex-2023-10-16".to_string());

        // Apply the transformer pipeline (same as in chat method)
        let pipeline = AuthSystemMessage::default()
            .when(|_| false) // Not using OAuth
            .pipe(CapitalizeToolNames)
            .pipe(DropInvalidToolUse)
            .pipe(SanitizeToolIds);

        let request = pipeline
            .pipe(RemoveOutputFormat)
            .pipe(SetCache)
            .transform(request);

        // Verify output_format is None for Vertex AI
        assert_eq!(
            request.output_format, None,
            "Vertex AI requests should not include output_format"
        );

        // Verify anthropic_version is set
        assert_eq!(
            request.anthropic_version,
            Some("vertex-2023-10-16".to_string()),
            "Vertex AI requests should include anthropic_version"
        );
    }
}
