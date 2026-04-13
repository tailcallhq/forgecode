use std::sync::Arc;

use anyhow::Context as _;
use forge_app::domain::{ChatCompletionMessage, Context, Model, ModelId, ResultStream};
use forge_app::dto::google::{EventData, Request};
use forge_app::{EnvironmentInfra, HttpInfra};
use forge_domain::{ChatRepository, Provider};
use reqwest::Url;
use tokio_stream::StreamExt;
use tracing::debug;

use crate::provider::event::into_chat_completion_message;
use crate::provider::retry::into_retry;
use crate::provider::utils::{create_headers, format_http_context};

#[derive(Clone)]
struct Google<T> {
    http: Arc<T>,
    api_key: String,
    chat_url: Url,
    models: forge_domain::ModelSource<Url>,
    use_api_key_header: bool,
}

impl<H: HttpInfra> Google<H> {
    pub fn new(
        http: Arc<H>,
        api_key: String,
        chat_url: Url,
        models: forge_domain::ModelSource<Url>,
        use_api_key_header: bool,
    ) -> Self {
        Self { http, api_key, chat_url, models, use_api_key_header }
    }

    fn get_headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];

        if self.use_api_key_header {
            headers.push(("x-goog-api-key".to_string(), self.api_key.clone()));
        } else {
            headers.push((
                "Authorization".to_string(),
                format!("Bearer {}", self.api_key),
            ));
        }

        headers
    }
}

impl<T: HttpInfra> Google<T> {
    pub async fn chat(
        &self,
        model: &ModelId,
        context: Context,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let request = Request::from(context);

        // Google models are specified in the URL path, not the request body
        // URL format: {base_url}/models/{model}:streamGenerateContent?alt=sse
        // The ?alt=sse query parameter is critical for proper SSE content-type
        let base_url = self.chat_url.as_str();
        let model_id_str = model.as_str();
        let model_name = model_id_str.strip_prefix("models/").unwrap_or(model_id_str);
        let full_url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            base_url.trim_end_matches('/'),
            model_name
        );
        let url = Url::parse(&full_url).with_context(|| "Failed to construct Google API URL")?;

        debug!(url = %url, model = %model, "Connecting Upstream");

        let json_bytes =
            serde_json::to_vec(&request).with_context(|| "Failed to serialize request")?;

        let source = self
            .http
            .http_eventsource(
                &url,
                Some(create_headers(self.get_headers())),
                json_bytes.into(),
            )
            .await
            .with_context(|| format_http_context(None, "POST", &url))?;

        let stream = into_chat_completion_message::<EventData>(url.clone(), source);

        Ok(Box::pin(stream))
    }

    pub async fn models(&self) -> anyhow::Result<Vec<Model>> {
        match &self.models {
            forge_domain::ModelSource::Url(url) => {
                debug!(url = %url, "Fetching models");

                let response = self
                    .http
                    .http_get(url, Some(create_headers(self.get_headers())))
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
                    // Google's models endpoint returns { "models": [...] }
                    #[derive(serde::Deserialize)]
                    struct ModelsResponse {
                        models: Vec<forge_app::dto::google::Model>,
                    }

                    let response: ModelsResponse = serde_json::from_str(&text)
                        .with_context(|| ctx_msg)
                        .with_context(|| "Failed to deserialize models response")?;
                    Ok(response.models.into_iter().map(Into::into).collect())
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

/// Repository for Google provider responses
pub struct GoogleResponseRepository<F> {
    infra: Arc<F>,
}

impl<F> GoogleResponseRepository<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

impl<F: HttpInfra> GoogleResponseRepository<F> {
    /// Creates a Google client from a provider configuration
    fn create_client(&self, provider: &Provider<Url>) -> anyhow::Result<Google<F>> {
        let chat_url = provider.url.clone();
        let models = provider
            .models
            .clone()
            .context("Google requires models configuration")?;
        let creds = provider
            .credential
            .as_ref()
            .context("Google provider requires credentials")?
            .auth_details
            .clone();

        // For Vertex AI, the Google ADC token is stored as ApiKey
        // For Vertex AI, the Google ADC token is stored as ApiKey
        // For OAuth, extract the access token
        let (token, use_api_key_header) = match creds {
            forge_domain::AuthDetails::ApiKey(api_key) => (api_key.as_str().to_string(), true),
            forge_domain::AuthDetails::GoogleAdc(token) => (token.as_str().to_string(), false),
            forge_domain::AuthDetails::OAuth { tokens, .. } => {
                (tokens.access_token.as_str().to_string(), false)
            }
            forge_domain::AuthDetails::OAuthWithApiKey { api_key, .. } => {
                (api_key.as_str().to_string(), true)
            }
        };

        Ok(Google::new(
            self.infra.clone(),
            token,
            chat_url,
            models,
            use_api_key_header,
        ))
    }
}
#[async_trait::async_trait]
impl<F: HttpInfra + EnvironmentInfra<Config = forge_config::ForgeConfig> + 'static> ChatRepository
    for GoogleResponseRepository<F>
{
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let retry_config = self.infra.get_config()?.retry.unwrap_or_default();
        let provider_client = self.create_client(&provider)?;

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
        let provider_client = self.create_client(&provider)?;

        provider_client
            .models()
            .await
            .map_err(|e| into_retry(e, &retry_config))
            .context("Failed to fetch models from Google provider")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use forge_app::HttpInfra;
    use forge_app::domain::{
        Context, ContextMessage, ToolCallFull, ToolCallId, ToolChoice, ToolName, ToolOutput,
        ToolResult,
    };
    use reqwest::header::HeaderMap;
    use reqwest_eventsource::EventSource;

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
            url: &Url,
            headers: Option<HeaderMap>,
            body: Bytes,
        ) -> anyhow::Result<EventSource> {
            let mut request = self.client.post(url.clone());
            if let Some(headers) = headers {
                request = request.headers(headers);
            }
            request = request.body(body);
            let request_builder = request;
            Ok(EventSource::new(request_builder).map_err(|e| anyhow::anyhow!(e))?)
        }
    }

    fn create_google(base_url: &str) -> anyhow::Result<Google<MockHttpClient>> {
        let chat_url = Url::parse(base_url)?;
        let model_url = Url::parse(base_url)?.join("models")?;
        Ok(Google::new(
            Arc::new(MockHttpClient::new()),
            "sk-test-key".to_string(),
            chat_url,
            forge_domain::ModelSource::Url(model_url),
            true,
        ))
    }

    fn create_mock_models_response() -> serde_json::Value {
        serde_json::json!({
            "models": [
                {
                    "name": "models/gemini-1.5-pro",
                    "version": "001",
                    "displayName": "Gemini 1.5 Pro",
                    "description": "Mid-size multimodal model that supports up to 1 million tokens",
                    "inputTokenLimit": 1000000,
                    "outputTokenLimit": 8192,
                    "supportedGenerationMethods": ["generateContent", "countTokens"],
                    "temperature": 1.0,
                    "topP": 0.95,
                    "topK": 64
                },
                {
                    "name": "models/gemini-1.5-flash",
                    "version": "001",
                    "displayName": "Gemini 1.5 Flash",
                    "description": "Fast and versatile multimodal model for scaling across diverse tasks",
                    "inputTokenLimit": 1000000,
                    "outputTokenLimit": 8192,
                    "supportedGenerationMethods": ["generateContent", "countTokens"],
                    "temperature": 1.0,
                    "topP": 0.95,
                    "topK": 64
                }
            ]
        })
    }

    fn create_error_response(message: &str, code: u16) -> serde_json::Value {
        serde_json::json!({
            "error": {
                "code": code,
                "message": message,
                "status": "PERMISSION_DENIED"
            }
        })
    }

    #[tokio::test]
    async fn test_url_for_models() {
        let chat_url = Url::parse("https://generativelanguage.googleapis.com/v1beta").unwrap();
        let model_url =
            Url::parse("https://generativelanguage.googleapis.com/v1beta/models").unwrap();
        let google = Google::new(
            Arc::new(MockHttpClient::new()),
            "sk-some-key".to_string(),
            chat_url,
            forge_domain::ModelSource::Url(model_url.clone()),
            true,
        );
        match &google.models {
            forge_domain::ModelSource::Url(url) => {
                assert_eq!(
                    url.as_str(),
                    "https://generativelanguage.googleapis.com/v1beta/models"
                );
            }
            _ => panic!("Expected Models::Url variant"),
        }
    }

    #[tokio::test]
    async fn test_request_conversion() {
        let model_id = ModelId::new("gemini-1.5-pro");
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

        // We can't easily test Request::from(context) directly here because Request is
        // private or we need to access it via Google::chat But we can check the
        // serialized request if we mock the http call. However, Request is pub
        // in dto::google::Request, so we can use it if we import it. The import
        // `use forge_app::dto::google::{EventData, Request};` is already there.

        let request = Request::from(context);
        insta::assert_json_snapshot!(request);
    }

    #[tokio::test]
    async fn test_fetch_models_success() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;
        let mock = fixture
            .mock_models(create_mock_models_response(), 200)
            .await;
        let google = create_google(&fixture.url())?;
        let actual = google.models().await?;

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
            .mock_models(create_error_response("Invalid API key", 400), 400)
            .await;

        let google = create_google(&fixture.url())?;
        let actual = google.models().await;

        mock.assert_async().await;

        // Verify that we got an error
        assert!(actual.is_err());
        insta::assert_snapshot!(normalize_ports(format!("{:#?}", actual.unwrap_err())));
        Ok(())
    }

    #[test]
    fn test_get_headers_with_api_key_header() {
        let chat_url = Url::parse("https://google.com").unwrap();
        let model_url = Url::parse("https://google.com/models").unwrap();
        let google = Google::new(
            Arc::new(MockHttpClient::new()),
            "sk-test-key".to_string(),
            chat_url,
            forge_domain::ModelSource::Url(model_url),
            true, // use_api_key_header = true
        );

        let headers = google.get_headers();

        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "x-goog-api-key" && v == "sk-test-key")
        );
        assert!(!headers.iter().any(|(k, _)| k == "Authorization"));
    }

    #[test]
    fn test_get_headers_with_bearer_token() {
        let chat_url = Url::parse("https://google.com").unwrap();
        let model_url = Url::parse("https://google.com/models").unwrap();
        let google = Google::new(
            Arc::new(MockHttpClient::new()),
            "oauth-token".to_string(),
            chat_url,
            forge_domain::ModelSource::Url(model_url),
            false, // use_api_key_header = false
        );

        let headers = google.get_headers();

        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "Authorization" && v == "Bearer oauth-token")
        );
        assert!(!headers.iter().any(|(k, _)| k == "x-goog-api-key"));
    }

    fn create_mock_chat_response(text: &str) -> String {
        serde_json::json!({
            "candidates": [
                {
                    "content": {
                        "parts": [
                            { "text": text }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP",
                    "index": 0
                }
            ],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5,
                "totalTokenCount": 15
            }
        })
        .to_string()
    }

    #[tokio::test]
    async fn test_chat_success() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;
        let model_id = "gemini-1.5-pro";

        let response1 = format!("data: {}", create_mock_chat_response("Hello"));
        let response2 = format!("data: {}", create_mock_chat_response(" World"));

        let mock = fixture
            .mock_google_chat_stream(model_id, vec![response1, response2], 200)
            .await;

        let google = create_google(&fixture.url())?;

        let context = Context::default().add_message(ContextMessage::user(
            "Hi",
            Some(ModelId::new(format!("models/{}", model_id))),
        ));

        let mut stream = google
            .chat(&ModelId::new(format!("models/{}", model_id)), context)
            .await?;

        let mut content = String::new();
        while let Some(result) = stream.next().await {
            let message = result?;
            if let Some(c) = message.content {
                content.push_str(c.as_str());
            }
        }

        mock.assert_async().await;

        assert_eq!(content, "Hello World");

        Ok(())
    }
}
