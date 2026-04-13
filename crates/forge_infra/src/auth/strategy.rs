use std::time::Duration;

use forge_app::{AuthStrategy, OAuthHttpProvider, StrategyFactory};
use forge_domain::{
    ApiKey, ApiKeyRequest, AuthContextRequest, AuthContextResponse, AuthCredential, CodeRequest,
    DeviceCodeRequest, OAuthConfig, OAuthTokenResponse, OAuthTokens, ProviderId, URLParamSpec,
};
use google_cloud_auth::credentials::Builder;
use oauth2::basic::BasicClient;
use oauth2::{ClientId, DeviceAuthorizationUrl, Scope, TokenUrl};
use reqwest::header::{HeaderMap, HeaderValue};
use url::Url;

use crate::auth::error::Error as AuthError;
use crate::auth::http::{AnthropicHttpProvider, GithubHttpProvider, StandardHttpProvider};
use crate::auth::util::*;

/// API Key Strategy - Simple static key authentication
pub struct ApiKeyStrategy {
    provider_id: ProviderId,
    required_params: Vec<URLParamSpec>,
}

impl ApiKeyStrategy {
    pub fn new(provider_id: ProviderId, required_params: Vec<URLParamSpec>) -> Self {
        Self { provider_id, required_params }
    }
}

#[async_trait::async_trait]
impl AuthStrategy for ApiKeyStrategy {
    async fn init(&self) -> anyhow::Result<AuthContextRequest> {
        Ok(AuthContextRequest::ApiKey(ApiKeyRequest {
            required_params: self.required_params.clone(),
            existing_params: None,
            api_key: None,
        }))
    }

    async fn complete(
        &self,
        context_response: AuthContextResponse,
    ) -> anyhow::Result<AuthCredential> {
        match context_response {
            AuthContextResponse::ApiKey(ctx) => Ok(AuthCredential::new_api_key(
                self.provider_id.clone(),
                ctx.response.api_key,
            )
            .url_params(ctx.response.url_params)),
            _ => Err(AuthError::InvalidContext("Expected ApiKey context".to_string()).into()),
        }
    }

    async fn refresh(&self, credential: &AuthCredential) -> anyhow::Result<AuthCredential> {
        // API keys don't expire - return as-is
        Ok(credential.clone())
    }
}

/// Extract the ChatGPT account ID from a JWT token's claims.
///
/// Checks `chatgpt_account_id`, `https://api.openai.com/auth.chatgpt_account_id`,
/// and `organizations[0].id` in that order, matching the opencode
/// implementation.
fn extract_chatgpt_account_id(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    use base64::Engine;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;

    // Try chatgpt_account_id first
    if let Some(id) = claims["chatgpt_account_id"].as_str() {
        return Some(id.to_string());
    }
    // Try nested auth claim
    if let Some(id) = claims["https://api.openai.com/auth"]["chatgpt_account_id"].as_str() {
        return Some(id.to_string());
    }
    // Fall back to organizations[0].id
    if let Some(id) = claims["organizations"]
        .as_array()
        .and_then(|orgs| orgs.first())
        .and_then(|org| org["id"].as_str())
    {
        return Some(id.to_string());
    }
    None
}

/// Adds Codex-specific credential metadata derived from OAuth tokens.
///
/// Tries to extract the account ID from the `id_token` first (which typically
/// contains the user identity claims in OpenID Connect flows), then falls back
/// to the `access_token` if needed.
fn enrich_codex_oauth_credential(
    provider_id: &ProviderId,
    credential: &mut AuthCredential,
    id_token: Option<&str>,
    access_token: &str,
) {
    if *provider_id != ProviderId::CODEX {
        return;
    }

    // Try id_token first (preferred for user identity claims)
    let account_id = id_token
        .and_then(extract_chatgpt_account_id)
        .or_else(|| extract_chatgpt_account_id(access_token));

    if let Some(account_id) = account_id {
        credential
            .url_params
            .insert("chatgpt_account_id".to_string().into(), account_id.into());
    }
}

/// OAuth Code Strategy - Browser redirect flow
pub struct OAuthCodeStrategy<T> {
    provider_id: ProviderId,
    config: OAuthConfig,
    adapter: T,
}

impl<T> OAuthCodeStrategy<T> {
    pub fn new(adapter: T, provider_id: ProviderId, config: OAuthConfig) -> Self {
        Self { config, provider_id, adapter }
    }
}

#[async_trait::async_trait]
impl<T: OAuthHttpProvider> AuthStrategy for OAuthCodeStrategy<T> {
    async fn init(&self) -> anyhow::Result<AuthContextRequest> {
        let auth_params = self
            .adapter
            .build_auth_url(&self.config)
            .await
            .map_err(|e| AuthError::InitiationFailed(format!("Failed to build auth URL: {e}")))?;

        Ok(AuthContextRequest::Code(CodeRequest {
            authorization_url: Url::parse(&auth_params.auth_url)?,
            state: auth_params.state.into(),
            pkce_verifier: auth_params.code_verifier.map(Into::into),
            oauth_config: self.config.clone(),
        }))
    }

    async fn complete(
        &self,
        context_response: AuthContextResponse,
    ) -> anyhow::Result<AuthCredential> {
        match context_response {
            AuthContextResponse::Code(ctx) => {
                let token_response = self
                    .adapter
                    .exchange_code(
                        &ctx.request.oauth_config,
                        ctx.response.code.as_str(),
                        ctx.request.pkce_verifier.as_ref().map(|v| v.as_str()),
                    )
                    .await
                    .map_err(|e| {
                        AuthError::CompletionFailed(format!(
                            "Failed to exchange authorization code: {e}"
                        ))
                    })?;

                let access_token = token_response.access_token.clone();
                let id_token = token_response.id_token.clone();
                let mut credential = build_oauth_credential(
                    self.provider_id.clone(),
                    token_response,
                    &ctx.request.oauth_config,
                    chrono::Duration::hours(1), // Code flow default
                )?;
                enrich_codex_oauth_credential(
                    &self.provider_id,
                    &mut credential,
                    id_token.as_deref(),
                    &access_token,
                );
                Ok(credential)
            }
            _ => Err(AuthError::InvalidContext("Expected Code context".to_string()).into()),
        }
    }

    async fn refresh(&self, credential: &AuthCredential) -> anyhow::Result<AuthCredential> {
        refresh_oauth_credential(
            credential,
            &self.config,
            chrono::Duration::hours(1),
            false, // No API key exchange
        )
        .await
    }
}

/// OAuth Device Strategy - Device code flow
pub struct OAuthDeviceStrategy {
    provider_id: ProviderId,
    config: OAuthConfig,
}

impl OAuthDeviceStrategy {
    pub fn new(provider_id: ProviderId, config: OAuthConfig) -> Self {
        Self { provider_id, config }
    }
}

#[async_trait::async_trait]
impl AuthStrategy for OAuthDeviceStrategy {
    async fn init(&self) -> anyhow::Result<AuthContextRequest> {
        // Build oauth2 client
        let client = BasicClient::new(ClientId::new(self.config.client_id.to_string()))
            .set_device_authorization_url(
                DeviceAuthorizationUrl::new(self.config.auth_url.to_string())
                    .map_err(|e| AuthError::InitiationFailed(format!("Invalid auth_url: {e}")))?,
            )
            .set_token_uri(
                TokenUrl::new(self.config.token_url.to_string())
                    .map_err(|e| AuthError::InitiationFailed(format!("Invalid token_url: {e}")))?,
            );

        // Request device authorization
        let mut request = client.exchange_device_code();
        for scope in &self.config.scopes {
            request = request.add_scope(Scope::new(scope.clone()));
        }

        // Build HTTP client with custom headers
        let http_client = build_http_client(self.config.custom_headers.as_ref()).map_err(|e| {
            AuthError::InitiationFailed(format!("Failed to build HTTP client: {e}"))
        })?;

        let http_fn = |req| github_compliant_http_request(http_client.clone(), req);

        let device_auth_response: oauth2::StandardDeviceAuthorizationResponse =
            request.request_async(&http_fn).await.map_err(|e| {
                AuthError::InitiationFailed(format!("Device authorization request failed: {e}"))
            })?;

        // Build the type-safe context
        Ok(AuthContextRequest::DeviceCode(DeviceCodeRequest {
            user_code: device_auth_response.user_code().secret().to_string().into(),
            device_code: device_auth_response
                .device_code()
                .secret()
                .to_string()
                .into(),
            verification_uri: Url::parse(&device_auth_response.verification_uri().to_string())?,
            verification_uri_complete: device_auth_response
                .verification_uri_complete()
                .map(|u| Url::parse(&u.secret().to_string()).unwrap()),
            expires_in: device_auth_response.expires_in().as_secs(),
            interval: device_auth_response.interval().as_secs(),
            oauth_config: self.config.clone(),
        }))
    }

    async fn complete(
        &self,
        context_response: AuthContextResponse,
    ) -> anyhow::Result<AuthCredential> {
        match context_response {
            AuthContextResponse::DeviceCode(ctx) => {
                let token_response = poll_for_tokens(
                    &ctx.request.device_code,
                    &self.config,
                    Duration::from_secs(600),
                    false,
                )
                .await?;

                build_oauth_credential(
                    self.provider_id.clone(),
                    token_response,
                    &self.config,
                    chrono::Duration::days(365), // Device flow default
                )
            }
            _ => Err(AuthError::InvalidContext("Expected DeviceCode context".to_string()).into()),
        }
    }

    async fn refresh(&self, credential: &AuthCredential) -> anyhow::Result<AuthCredential> {
        refresh_oauth_credential(
            credential,
            &self.config,
            chrono::Duration::days(30),
            false, // No API key exchange
        )
        .await
    }
}

/// OAuth-with-API-Key Strategy - Hybrid flow (GitHub Copilot pattern)
pub struct OAuthWithApiKeyStrategy {
    provider_id: ProviderId,
    oauth_config: OAuthConfig,
    api_key_exchange_url: Url,
}

impl OAuthWithApiKeyStrategy {
    pub fn new(provider_id: ProviderId, oauth_config: OAuthConfig) -> anyhow::Result<Self> {
        let api_key_exchange_url = oauth_config
            .token_refresh_url
            .clone()
            .ok_or_else(|| AuthError::InitiationFailed("Missing token_refresh_url".to_string()))?;

        Ok(Self { provider_id, oauth_config, api_key_exchange_url })
    }
}

#[async_trait::async_trait]
impl AuthStrategy for OAuthWithApiKeyStrategy {
    async fn init(&self) -> anyhow::Result<AuthContextRequest> {
        // Same as OAuth Device init
        let client = BasicClient::new(ClientId::new(self.oauth_config.client_id.to_string()))
            .set_device_authorization_url(
                DeviceAuthorizationUrl::new(self.oauth_config.auth_url.to_string())
                    .map_err(|e| AuthError::InitiationFailed(format!("Invalid auth_url: {e}")))?,
            )
            .set_token_uri(
                TokenUrl::new(self.oauth_config.token_url.to_string())
                    .map_err(|e| AuthError::InitiationFailed(format!("Invalid token_url: {e}")))?,
            );

        let mut request = client.exchange_device_code();
        for scope in &self.oauth_config.scopes {
            request = request.add_scope(Scope::new(scope.clone()));
        }

        let http_client =
            build_http_client(self.oauth_config.custom_headers.as_ref()).map_err(|e| {
                AuthError::InitiationFailed(format!("Failed to build HTTP client: {e}"))
            })?;

        let http_fn = |req| github_compliant_http_request(http_client.clone(), req);

        let device_auth_response: oauth2::StandardDeviceAuthorizationResponse =
            request.request_async(&http_fn).await.map_err(|e| {
                AuthError::InitiationFailed(format!("Device authorization request failed: {e}"))
            })?;

        Ok(AuthContextRequest::DeviceCode(DeviceCodeRequest {
            user_code: device_auth_response.user_code().secret().to_string().into(),
            device_code: device_auth_response
                .device_code()
                .secret()
                .to_string()
                .into(),
            verification_uri: Url::parse(&device_auth_response.verification_uri().to_string())?,
            verification_uri_complete: device_auth_response
                .verification_uri_complete()
                .map(|u| Url::parse(&u.secret().to_string()).unwrap()),
            expires_in: device_auth_response.expires_in().as_secs(),
            interval: device_auth_response.interval().as_secs(),
            oauth_config: self.oauth_config.clone(),
        }))
    }

    async fn complete(
        &self,
        context_response: AuthContextResponse,
    ) -> anyhow::Result<AuthCredential> {
        match context_response {
            AuthContextResponse::DeviceCode(ctx) => {
                // Poll for OAuth tokens (GitHub-compatible)
                let token_response = poll_for_tokens(
                    &ctx.request.device_code,
                    &self.oauth_config,
                    Duration::from_secs(600),
                    true,
                )
                .await?;

                // Exchange for API key
                let (api_key, expires_at) = exchange_oauth_for_api_key(
                    &token_response.access_token,
                    &self.api_key_exchange_url,
                    &self.oauth_config,
                )
                .await?;

                let oauth_tokens = OAuthTokens::new(
                    token_response.access_token,
                    token_response.refresh_token,
                    expires_at,
                );

                Ok(AuthCredential::new_oauth_with_api_key(
                    self.provider_id.clone(),
                    oauth_tokens,
                    api_key,
                    self.oauth_config.clone(),
                ))
            }
            _ => Err(AuthError::InvalidContext("Expected DeviceCode context".to_string()).into()),
        }
    }

    async fn refresh(&self, credential: &AuthCredential) -> anyhow::Result<AuthCredential> {
        refresh_oauth_credential(
            credential,
            &self.oauth_config,
            chrono::Duration::hours(1), // Unused for API key flow
            true,                       // WITH API key exchange
        )
        .await
    }
}

/// Google Application Default Credentials (ADC) Strategy
/// Uses Google Cloud SDK's ADC mechanism with automatic token refresh
pub struct GoogleAdcStrategy {
    provider_id: ProviderId,
    required_params: Vec<URLParamSpec>,
}

impl GoogleAdcStrategy {
    pub fn new(provider_id: ProviderId, required_params: Vec<URLParamSpec>) -> Self {
        Self { provider_id, required_params }
    }
}

#[async_trait::async_trait]
impl AuthStrategy for GoogleAdcStrategy {
    async fn init(&self) -> anyhow::Result<AuthContextRequest> {
        // For Google ADC, we don't need any user interaction for the API key
        // The credentials are automatically discovered from:
        // 1. GOOGLE_APPLICATION_CREDENTIALS env var (service account)
        // 2. gcloud ADC credentials (user credentials)
        // 3. Metadata server (GCP environment)
        // However, we still need to collect URL params like PROJECT_ID and LOCATION
        Ok(AuthContextRequest::ApiKey(ApiKeyRequest {
            required_params: self.required_params.clone(),
            existing_params: None,
            api_key: Some("google_adc_marker".to_string().into()), // Marker to indicate ADC usage
        }))
    }

    async fn complete(
        &self,
        context_response: AuthContextResponse,
    ) -> anyhow::Result<AuthCredential> {
        match context_response {
            AuthContextResponse::ApiKey(ctx) => {
                // Validate that gcloud auth is properly configured before completing
                // authentication This ensures the user has run 'gcloud auth
                // application-default login'
                use google_cloud_auth::credentials::Builder;
                const VERTEX_AI_SCOPES: &[&str] =
                    &["https://www.googleapis.com/auth/cloud-platform"];
                let credentials = Builder::default()
                    .with_scopes(VERTEX_AI_SCOPES.iter().map(|s| s.to_string()))
                    .build_access_token_credentials()
                    .map_err(|e| {
                        AuthError::CompletionFailed(format!(
                            "Google ADC not configured: {e}. Please run 'gcloud auth application-default login' to set up credentials."
                        ))
                    })?;

                // Try to fetch a token to verify authentication works
                credentials
                    .access_token()
                    .await
                    .map_err(|e| {
                        AuthError::CompletionFailed(format!(
                            "Failed to obtain access token: {e}. Your ADC credentials may be expired — run 'gcloud auth application-default login' to re-authenticate."
                        ))
                    })?;

                // For Google ADC, we save a marker instead of the actual token
                // The token will be refreshed on every use
                // But we still need to save the url_params (PROJECT_ID, LOCATION)
                Ok(AuthCredential::new_google_adc(
                    self.provider_id.clone(),
                    ApiKey::from("google_adc_marker".to_string()), /* Marker that will trigger
                                                                    * refresh */
                )
                .url_params(ctx.response.url_params))
            }
            _ => Err(AuthError::InvalidContext("Expected ApiKey context".to_string()).into()),
        }
    }

    async fn refresh(&self, credential: &AuthCredential) -> anyhow::Result<AuthCredential> {
        // Google ADC handles token refresh automatically
        // We just need to get a fresh token using the Builder API
        // Vertex AI requires the cloud-platform scope
        const VERTEX_AI_SCOPES: &[&str] = &["https://www.googleapis.com/auth/cloud-platform"];
        let credentials = Builder::default()
            .with_scopes(VERTEX_AI_SCOPES.iter().map(|s| s.to_string()))
            .build_access_token_credentials()
            .map_err(|e| {
                AuthError::RefreshFailed(format!(
                    "Failed to create Google credentials builder: {e}"
                ))
            })?;

        let access_token = credentials.access_token().await.map_err(|e| {
            AuthError::RefreshFailed(format!(
                "Failed to refresh Google access token: {e}. Your ADC credentials may be expired — run 'gcloud auth application-default login' to re-authenticate."
            ))
        })?;

        Ok(AuthCredential::new_google_adc(
            self.provider_id.clone(),
            ApiKey::from(access_token.token),
        )
        .url_params(credential.url_params.clone()))
    }
}

/// OpenAI Codex Device Strategy - Custom device auth for ChatGPT Pro/Plus
///
/// Implements the OpenAI-specific device authorization flow used by Codex:
/// 1. Request device code from `/api/accounts/deviceauth/usercode`
/// 2. User enters code at `https://auth.openai.com/codex/device`
/// 3. Poll `/api/accounts/deviceauth/token` for authorization code + verifier
/// 4. Exchange authorization code for OAuth tokens via standard token endpoint
pub struct CodexDeviceStrategy {
    provider_id: ProviderId,
    config: OAuthConfig,
}

impl CodexDeviceStrategy {
    pub fn new(provider_id: ProviderId, config: OAuthConfig) -> Self {
        Self { provider_id, config }
    }
}

/// Response from the OpenAI device auth usercode endpoint
#[derive(Debug, serde::Deserialize)]
struct CodexDeviceAuthResponse {
    device_auth_id: String,
    user_code: String,
    interval: String,
}

/// Response from the OpenAI device auth token polling endpoint
#[derive(Debug, serde::Deserialize)]
struct CodexDeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

#[async_trait::async_trait]
impl AuthStrategy for CodexDeviceStrategy {
    async fn init(&self) -> anyhow::Result<AuthContextRequest> {
        let http_client = build_http_client(self.config.custom_headers.as_ref()).map_err(|e| {
            AuthError::InitiationFailed(format!("Failed to build HTTP client: {e}"))
        })?;

        // Step 1: Request device authorization from OpenAI's custom endpoint
        let response = http_client
            .post(self.config.auth_url.as_str())
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "client_id": self.config.client_id.as_str()
            }))
            .send()
            .await
            .map_err(|e| {
                AuthError::InitiationFailed(format!("Device authorization request failed: {e}"))
            })?;

        if !response.status().is_success() {
            return Err(AuthError::InitiationFailed(format!(
                "Device authorization failed with status: {}",
                response.status()
            ))
            .into());
        }

        let device_data: CodexDeviceAuthResponse = response.json().await.map_err(|e| {
            AuthError::InitiationFailed(format!("Failed to parse device auth response: {e}"))
        })?;

        let interval: u64 = device_data.interval.parse().unwrap_or(5).max(1);

        // Build the device code request using existing domain types
        // We encode the device_auth_id in the device_code field
        Ok(AuthContextRequest::DeviceCode(DeviceCodeRequest {
            user_code: device_data.user_code.clone().into(),
            device_code: device_data.device_auth_id.into(),
            verification_uri: Url::parse("https://auth.openai.com/codex/device")?,
            verification_uri_complete: None,
            expires_in: 300, // 5 minute timeout
            interval,
            oauth_config: self.config.clone(),
        }))
    }

    async fn complete(
        &self,
        context_response: AuthContextResponse,
    ) -> anyhow::Result<AuthCredential> {
        match context_response {
            AuthContextResponse::DeviceCode(ctx) => {
                // Poll for authorization code using the custom OpenAI endpoint
                let token_response = codex_poll_for_tokens(&ctx.request, &self.config).await?;

                let access_token = token_response.access_token.clone();
                let id_token = token_response.id_token.clone();
                let mut credential = build_oauth_credential(
                    self.provider_id.clone(),
                    token_response,
                    &self.config,
                    chrono::Duration::hours(1),
                )?;

                // Store account_id in url_params so it's persisted and available
                // for chat request headers.
                enrich_codex_oauth_credential(
                    &self.provider_id,
                    &mut credential,
                    id_token.as_deref(),
                    &access_token,
                );

                Ok(credential)
            }
            _ => Err(AuthError::InvalidContext("Expected DeviceCode context".to_string()).into()),
        }
    }

    async fn refresh(&self, credential: &AuthCredential) -> anyhow::Result<AuthCredential> {
        refresh_oauth_credential(credential, &self.config, chrono::Duration::hours(1), false).await
    }
}

/// Refresh OAuth credential - handles all OAuth flows
async fn refresh_oauth_credential(
    credential: &AuthCredential,
    config: &OAuthConfig,
    expiry_duration: chrono::Duration,
    with_api_key_exchange: bool,
) -> anyhow::Result<AuthCredential> {
    // Extract tokens (works for OAuth and OAuthWithApiKey)
    let tokens = extract_oauth_tokens(credential)?;

    // Determine which OAuth access token to use
    let (oauth_access_token, oauth_refresh_token) =
        if let Some(refresh_token) = &tokens.refresh_token {
            // If we have a refresh token, refresh the OAuth access token first
            tracing::debug!("Refreshing OAuth access token using refresh token");
            let token_response = refresh_access_token(config, refresh_token.as_str()).await?;
            (
                token_response.access_token.clone(),
                token_response.refresh_token,
            )
        } else {
            // No refresh token - use the existing long-lived OAuth access token
            // This is typical for GitHub Copilot where the OAuth token is long-lived
            tracing::debug!("No refresh token available, using existing OAuth access token");
            (
                tokens.access_token.to_string(),
                tokens.refresh_token.clone().map(|t| t.to_string()),
            )
        };

    // Exchange for API key if needed (GitHub Copilot pattern)
    let (api_key, expires_at) = if with_api_key_exchange {
        let url = config.token_refresh_url.as_ref().ok_or_else(|| {
            AuthError::RefreshFailed("Missing token_refresh_url for API key exchange".to_string())
        })?;
        let (key, expiry) = exchange_oauth_for_api_key(&oauth_access_token, url, config).await?;
        (Some(key), expiry)
    } else {
        let expiry = calculate_token_expiry(None, expiry_duration);
        (None, expiry)
    };

    // Build new tokens with refreshed OAuth access token
    let new_tokens = OAuthTokens::new(oauth_access_token, oauth_refresh_token, expires_at);

    // Build appropriate credential type while preserving URL params
    let refreshed = if let Some(key) = api_key {
        AuthCredential::new_oauth_with_api_key(
            credential.id.clone(),
            new_tokens,
            key,
            config.clone(),
        )
    } else {
        AuthCredential::new_oauth(credential.id.clone(), new_tokens, config.clone())
    };

    Ok(refreshed.url_params(credential.url_params.clone()))
}

/// Poll for OAuth tokens during device flow
async fn poll_for_tokens(
    device_code: &forge_domain::DeviceCode,
    config: &OAuthConfig,
    timeout: Duration,
    github_compatible: bool,
) -> anyhow::Result<OAuthTokenResponse> {
    let http_client = build_http_client(config.custom_headers.as_ref())
        .map_err(|e| AuthError::PollFailed(format!("Failed to build HTTP client: {e}")))?;

    let start_time = tokio::time::Instant::now();
    let interval = Duration::from_secs(5);

    loop {
        // Check timeout
        if start_time.elapsed() >= timeout {
            return Err(AuthError::Timeout(timeout).into());
        }

        // Sleep before polling (GitHub pattern only)
        if github_compatible {
            tokio::time::sleep(interval).await;
        }

        // Build token request
        let params = vec![
            (
                "grant_type".to_string(),
                "urn:ietf:params:oauth:grant-type:device_code".to_string(),
            ),
            ("device_code".to_string(), device_code.to_string()),
            ("client_id".to_string(), config.client_id.to_string()),
        ];

        let body = serde_urlencoded::to_string(&params)
            .map_err(|e| AuthError::PollFailed(format!("Failed to encode request: {e}")))?;

        // Make HTTP request with headers
        let mut headers = HeaderMap::new();
        headers.insert(
            "Content-Type",
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        headers.insert("Accept", HeaderValue::from_static("application/json"));

        inject_custom_headers(&mut headers, &config.custom_headers);

        let response = http_client
            .post(config.token_url.as_str())
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| AuthError::PollFailed(format!("HTTP request failed: {e}")))?;

        let status = response.status();
        let body_text = response
            .text()
            .await
            .map_err(|e| AuthError::PollFailed(format!("Failed to read response: {e}")))?;

        // GitHub-compatible: HTTP 200 can contain either success or error
        if github_compatible && status.is_success() {
            let token_response: serde_json::Value = serde_json::from_str(&body_text)
                .unwrap_or_else(|_| serde_json::json!({"error": "parse_error"}));

            // Check for error field first
            if let Some(error) = token_response["error"].as_str() {
                if handle_oauth_error(error).is_ok() {
                    // Retryable error - continue polling
                    continue;
                }
                // Terminal error - propagate
                return Err(handle_oauth_error(error).unwrap_err().into());
            }

            // No error field - parse as success
            let (access_token, refresh_token, expires_in) = parse_token_response(&body_text)?;

            return Ok(build_token_response(
                access_token,
                refresh_token,
                expires_in,
            ));
        }

        // Standard OAuth: HTTP success means tokens
        if !github_compatible && status.is_success() {
            let (access_token, refresh_token, expires_in) = parse_token_response(&body_text)?;
            return Ok(build_token_response(
                access_token,
                refresh_token,
                expires_in,
            ));
        }

        // Handle error responses (non-200 status for standard OAuth)
        let error_response: serde_json::Value = serde_json::from_str(&body_text)
            .unwrap_or_else(|_| serde_json::json!({"error": "unknown_error"}));

        if let Some(error) = error_response["error"].as_str() {
            if handle_oauth_error(error).is_ok() {
                // Retryable error - sleep and continue
                tokio::time::sleep(if error == "slow_down" {
                    interval * 2
                } else {
                    interval
                })
                .await;
                continue;
            }
            // Terminal error - propagate
            return Err(handle_oauth_error(error).unwrap_err().into());
        }

        // Unknown error
        return Err(AuthError::PollFailed(format!("HTTP {status}: {body_text}")).into());
    }
}

/// Poll for Codex tokens using OpenAI's custom device auth endpoints.
///
/// This differs from standard OAuth2 device code flow:
/// 1. Polls `/api/accounts/deviceauth/token` with `device_auth_id` +
///    `user_code`
/// 2. Receives `authorization_code` + `code_verifier` (not tokens directly)
/// 3. Exchanges the authorization code for OAuth tokens via standard token
///    endpoint
async fn codex_poll_for_tokens(
    request: &DeviceCodeRequest,
    config: &OAuthConfig,
) -> anyhow::Result<OAuthTokenResponse> {
    let http_client = build_http_client(config.custom_headers.as_ref())
        .map_err(|e| AuthError::PollFailed(format!("Failed to build HTTP client: {e}")))?;

    let timeout = Duration::from_secs(request.expires_in);
    let interval = Duration::from_secs(request.interval.max(1));
    // Add a safety margin to polling interval to avoid rate limiting
    let poll_interval = interval + Duration::from_secs(3);

    let start_time = tokio::time::Instant::now();

    // The auth_url in config points to the usercode endpoint; derive the token
    // polling endpoint from the same base
    let poll_url = config.auth_url.as_str().replace("/usercode", "/token");

    loop {
        if start_time.elapsed() >= timeout {
            return Err(AuthError::Timeout(timeout).into());
        }

        // Poll request and response handling would be here

        tokio::time::sleep(poll_interval).await;

        let response = http_client
            .post(&poll_url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "device_auth_id": request.device_code.as_str(),
                "user_code": request.user_code.as_str(),
            }))
            .send()
            .await
            .map_err(|e| AuthError::PollFailed(format!("HTTP request failed: {e}")))?;

        let status = response.status();

        if status.is_success() {
            // Parse the custom response containing authorization_code + code_verifier
            let device_token: CodexDeviceTokenResponse = response.json().await.map_err(|e| {
                AuthError::PollFailed(format!("Failed to parse device token response: {e}"))
            })?;

            // Exchange the authorization code for OAuth tokens via standard
            // endpoint. Use a clean HTTP client without custom headers since the
            // standard OAuth token endpoint rejects unknown headers.
            let clean_client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| AuthError::PollFailed(format!("Failed to build HTTP client: {e}")))?;

            let token_response = clean_client
                .post(config.token_url.as_str())
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(
                    serde_urlencoded::to_string([
                        ("grant_type", "authorization_code"),
                        ("code", &device_token.authorization_code),
                        (
                            "redirect_uri",
                            "https://auth.openai.com/deviceauth/callback",
                        ),
                        ("client_id", config.client_id.as_ref()),
                        ("code_verifier", &device_token.code_verifier),
                    ])
                    .map_err(|e| {
                        AuthError::PollFailed(format!("Failed to encode token request: {e}"))
                    })?,
                )
                .send()
                .await
                .map_err(|e| {
                    AuthError::PollFailed(format!("Token exchange request failed: {e}"))
                })?;

            if !token_response.status().is_success() {
                let token_exchange_status = token_response.status();
                let error_text = token_response.text().await.unwrap_or_default();
                return Err(AuthError::PollFailed(format!(
                    "Token exchange failed ({}): {}",
                    token_exchange_status, error_text
                ))
                .into());
            }

            let (access_token, refresh_token, expires_in) =
                parse_token_response(&token_response.text().await.map_err(|e| {
                    AuthError::PollFailed(format!("Failed to read token response: {e}"))
                })?)?;

            return Ok(build_token_response(
                access_token,
                refresh_token,
                expires_in,
            ));
        }

        // 403/404 means authorization pending (user hasn't entered code yet)
        if status.as_u16() == 403 || status.as_u16() == 404 {
            continue;
        }

        // Any other error is terminal
        let body_text = response.text().await.unwrap_or_default();
        return Err(AuthError::PollFailed(format!("HTTP {status}: {body_text}")).into());
    }
}

/// Exchange OAuth token for API key (GitHub Copilot pattern)
async fn exchange_oauth_for_api_key(
    oauth_token: &str,
    api_key_exchange_url: &Url,
    config: &OAuthConfig,
) -> anyhow::Result<(ApiKey, chrono::DateTime<chrono::Utc>)> {
    // Build request headers
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!("Bearer {oauth_token}")).map_err(|e| {
            AuthError::CompletionFailed(format!("Invalid authorization header: {e}"))
        })?,
    );

    // Add custom headers from config
    inject_custom_headers(&mut headers, &config.custom_headers);

    let response = build_http_client(config.custom_headers.as_ref())
        .map_err(|e| AuthError::CompletionFailed(format!("Failed to build HTTP client: {e}")))?
        .get(api_key_exchange_url.as_str())
        .headers(headers)
        .send()
        .await
        .map_err(|e| {
            AuthError::CompletionFailed(format!("API key exchange request failed: {e}"))
        })?;

    let status = response.status();
    if !status.is_success() {
        if status.as_u16() == 403 {
            return Err(AuthError::CompletionFailed(
                "Access denied. Ensure you have an active subscription.".to_string(),
            )
            .into());
        }
        return Err(AuthError::CompletionFailed(format!(
            "API key fetch failed ({}): {}",
            status,
            response.text().await.unwrap_or_default()
        ))
        .into());
    }

    let OAuthTokenResponse { access_token, expires_at, .. } =
        response.json().await.map_err(|e| {
            AuthError::CompletionFailed(format!("Failed to parse API key response: {e}"))
        })?;

    Ok((
        access_token.into(),
        chrono::DateTime::from_timestamp(expires_at.unwrap_or(0), 0)
            .unwrap_or_else(chrono::Utc::now),
    ))
}

/// Enum wrapper for all strategy implementations
/// Eliminates heap allocation and dynamic dispatch
pub enum AnyAuthStrategy {
    ApiKey(ApiKeyStrategy),
    OAuthCodeStandard(OAuthCodeStrategy<StandardHttpProvider>),
    OAuthCodeAnthropic(OAuthCodeStrategy<AnthropicHttpProvider>),
    OAuthCodeGithub(OAuthCodeStrategy<GithubHttpProvider>),
    OAuthDevice(OAuthDeviceStrategy),
    OAuthWithApiKey(OAuthWithApiKeyStrategy),
    GoogleAdc(GoogleAdcStrategy),
    CodexDevice(CodexDeviceStrategy),
}

#[async_trait::async_trait]
impl AuthStrategy for AnyAuthStrategy {
    async fn init(&self) -> anyhow::Result<AuthContextRequest> {
        match self {
            Self::ApiKey(s) => s.init().await,
            Self::OAuthCodeStandard(s) => s.init().await,
            Self::OAuthCodeAnthropic(s) => s.init().await,
            Self::OAuthCodeGithub(s) => s.init().await,
            Self::OAuthDevice(s) => s.init().await,
            Self::OAuthWithApiKey(s) => s.init().await,
            Self::GoogleAdc(s) => s.init().await,
            Self::CodexDevice(s) => s.init().await,
        }
    }

    async fn complete(
        &self,
        context_response: AuthContextResponse,
    ) -> anyhow::Result<AuthCredential> {
        match self {
            Self::ApiKey(s) => s.complete(context_response).await,
            Self::OAuthCodeStandard(s) => s.complete(context_response).await,
            Self::OAuthCodeAnthropic(s) => s.complete(context_response).await,
            Self::OAuthCodeGithub(s) => s.complete(context_response).await,
            Self::OAuthDevice(s) => s.complete(context_response).await,
            Self::OAuthWithApiKey(s) => s.complete(context_response).await,
            Self::GoogleAdc(s) => s.complete(context_response).await,
            Self::CodexDevice(s) => s.complete(context_response).await,
        }
    }

    async fn refresh(&self, credential: &AuthCredential) -> anyhow::Result<AuthCredential> {
        match self {
            Self::ApiKey(s) => s.refresh(credential).await,
            Self::OAuthCodeStandard(s) => s.refresh(credential).await,
            Self::OAuthCodeAnthropic(s) => s.refresh(credential).await,
            Self::OAuthCodeGithub(s) => s.refresh(credential).await,
            Self::OAuthDevice(s) => s.refresh(credential).await,
            Self::OAuthWithApiKey(s) => s.refresh(credential).await,
            Self::GoogleAdc(s) => s.refresh(credential).await,
            Self::CodexDevice(s) => s.refresh(credential).await,
        }
    }
}

/// Factory for creating authentication strategies
pub struct ForgeAuthStrategyFactory;

impl Default for ForgeAuthStrategyFactory {
    fn default() -> Self {
        Self
    }
}

impl ForgeAuthStrategyFactory {
    pub fn new(_environment: forge_domain::Environment) -> Self {
        Self
    }
}

impl StrategyFactory for ForgeAuthStrategyFactory {
    type Strategy = AnyAuthStrategy;

    fn create_auth_strategy(
        &self,
        provider_id: ProviderId,
        auth_method: forge_domain::AuthMethod,
        required_params: Vec<URLParamSpec>,
    ) -> anyhow::Result<Self::Strategy> {
        match auth_method {
            forge_domain::AuthMethod::ApiKey => Ok(AnyAuthStrategy::ApiKey(ApiKeyStrategy::new(
                provider_id,
                required_params,
            ))),
            forge_domain::AuthMethod::OAuthCode(config) => {
                if provider_id == ProviderId::CLAUDE_CODE {
                    return Ok(AnyAuthStrategy::OAuthCodeAnthropic(OAuthCodeStrategy::new(
                        AnthropicHttpProvider,
                        provider_id,
                        config,
                    )));
                }

                if provider_id == ProviderId::GITHUB_COPILOT {
                    return Ok(AnyAuthStrategy::OAuthCodeGithub(OAuthCodeStrategy::new(
                        GithubHttpProvider,
                        provider_id,
                        config,
                    )));
                }

                Ok(AnyAuthStrategy::OAuthCodeStandard(OAuthCodeStrategy::new(
                    StandardHttpProvider,
                    provider_id,
                    config,
                )))
            }
            forge_domain::AuthMethod::OAuthDevice(config) => {
                // Check if this is OAuth-with-API-Key flow (GitHub Copilot pattern)
                if config.token_refresh_url.is_some() {
                    Ok(AnyAuthStrategy::OAuthWithApiKey(
                        OAuthWithApiKeyStrategy::new(provider_id, config)?,
                    ))
                } else {
                    Ok(AnyAuthStrategy::OAuthDevice(OAuthDeviceStrategy::new(
                        provider_id,
                        config,
                    )))
                }
            }
            forge_domain::AuthMethod::GoogleAdc => Ok(AnyAuthStrategy::GoogleAdc(
                GoogleAdcStrategy::new(provider_id, required_params),
            )),
            forge_domain::AuthMethod::CodexDevice(config) => Ok(AnyAuthStrategy::CodexDevice(
                CodexDeviceStrategy::new(provider_id, config),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use forge_domain::URLParam;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_create_auth_strategy_api_key() {
        let factory = ForgeAuthStrategyFactory;
        let strategy = factory.create_auth_strategy(
            ProviderId::OPENAI,
            forge_domain::AuthMethod::ApiKey,
            vec![],
        );
        assert!(strategy.is_ok());
    }

    #[test]
    fn test_create_auth_strategy_oauth_code() {
        let config = OAuthConfig {
            client_id: "test".to_string().into(),
            auth_url: Url::parse("https://example.com/auth").unwrap(),
            token_url: Url::parse("https://example.com/token").unwrap(),
            scopes: vec![],
            redirect_uri: None,
            use_pkce: false,
            token_refresh_url: None,
            extra_auth_params: None,
            custom_headers: None,
        };

        let factory = ForgeAuthStrategyFactory;
        let strategy = factory.create_auth_strategy(
            ProviderId::OPENAI,
            forge_domain::AuthMethod::OAuthCode(config),
            vec![],
        );
        assert!(strategy.is_ok());
    }

    #[test]
    fn test_create_auth_strategy_oauth_device() {
        let config = OAuthConfig {
            client_id: "test".to_string().into(),
            auth_url: Url::parse("https://example.com/auth").unwrap(),
            token_url: Url::parse("https://example.com/token").unwrap(),
            scopes: vec![],
            redirect_uri: None,
            use_pkce: false,
            token_refresh_url: None,
            extra_auth_params: None,
            custom_headers: None,
        };

        let factory = ForgeAuthStrategyFactory;
        let strategy = factory.create_auth_strategy(
            ProviderId::OPENAI,
            forge_domain::AuthMethod::OAuthDevice(config),
            vec![],
        );
        assert!(strategy.is_ok());
    }

    #[test]
    fn test_create_auth_strategy_oauth_with_api_key() {
        let config = OAuthConfig {
            client_id: "test".to_string().into(),
            auth_url: Url::parse("https://example.com/auth").unwrap(),
            token_url: Url::parse("https://example.com/token").unwrap(),
            scopes: vec![],
            redirect_uri: None,
            use_pkce: false,
            token_refresh_url: Some(Url::parse("https://example.com/api_key").unwrap()),
            extra_auth_params: None,
            custom_headers: None,
        };

        let factory = ForgeAuthStrategyFactory;
        let strategy = factory.create_auth_strategy(
            ProviderId::GITHUB_COPILOT,
            forge_domain::AuthMethod::OAuthDevice(config),
            vec![],
        );
        assert!(strategy.is_ok());
    }

    #[test]
    fn test_create_auth_strategy_codex_device() {
        let config = OAuthConfig {
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann".to_string().into(),
            auth_url: Url::parse("https://auth.openai.com/api/accounts/deviceauth/usercode")
                .unwrap(),
            token_url: Url::parse("https://auth.openai.com/oauth/token").unwrap(),
            scopes: vec![],
            redirect_uri: None,
            use_pkce: false,
            token_refresh_url: None,
            extra_auth_params: None,
            custom_headers: None,
        };

        let factory = ForgeAuthStrategyFactory;
        let actual = factory.create_auth_strategy(
            ProviderId::CODEX,
            forge_domain::AuthMethod::CodexDevice(config),
            vec![],
        );
        assert!(actual.is_ok());
        assert!(matches!(actual.unwrap(), AnyAuthStrategy::CodexDevice(_)));
    }

    /// Helper to build a JWT token with the given claims payload.
    fn build_jwt(claims: &serde_json::Value) -> String {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(claims).unwrap());
        format!("{header}.{payload}.fake_signature")
    }

    #[test]
    fn test_extract_chatgpt_account_id_from_direct_claim() {
        let fixture = build_jwt(&serde_json::json!({
            "chatgpt_account_id": "acct_123"
        }));
        let actual = extract_chatgpt_account_id(&fixture);
        let expected = Some("acct_123".to_string());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_chatgpt_account_id_from_nested_claim() {
        let fixture = build_jwt(&serde_json::json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_nested_456"
            }
        }));
        let actual = extract_chatgpt_account_id(&fixture);
        let expected = Some("acct_nested_456".to_string());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_chatgpt_account_id_from_organizations() {
        let fixture = build_jwt(&serde_json::json!({
            "organizations": [
                {"id": "org_789", "name": "My Org"}
            ]
        }));
        let actual = extract_chatgpt_account_id(&fixture);
        let expected = Some("org_789".to_string());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_chatgpt_account_id_prefers_direct_claim() {
        let fixture = build_jwt(&serde_json::json!({
            "chatgpt_account_id": "direct",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "nested"
            },
            "organizations": [{"id": "org"}]
        }));
        let actual = extract_chatgpt_account_id(&fixture);
        let expected = Some("direct".to_string());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_chatgpt_account_id_returns_none_for_empty_claims() {
        let fixture = build_jwt(&serde_json::json!({}));
        let actual = extract_chatgpt_account_id(&fixture);
        assert_eq!(actual, None);
    }

    #[test]
    fn test_extract_chatgpt_account_id_returns_none_for_invalid_jwt() {
        let actual = extract_chatgpt_account_id("not-a-jwt");
        assert_eq!(actual, None);
    }

    #[test]
    fn test_extract_chatgpt_account_id_returns_none_for_invalid_base64() {
        let actual = extract_chatgpt_account_id("header.!!!invalid-base64!!!.signature");
        assert_eq!(actual, None);
    }

    #[test]
    fn test_extract_chatgpt_account_id_returns_none_for_empty_organizations() {
        let fixture = build_jwt(&serde_json::json!({
            "organizations": []
        }));
        let actual = extract_chatgpt_account_id(&fixture);
        assert_eq!(actual, None);
    }

    #[tokio::test]
    async fn test_refresh_oauth_credential_preserves_url_params() {
        let fixture_config = OAuthConfig {
            client_id: "test".to_string().into(),
            auth_url: Url::parse("https://example.com/auth").unwrap(),
            token_url: Url::parse("https://example.com/token").unwrap(),
            scopes: vec![],
            redirect_uri: None,
            use_pkce: false,
            token_refresh_url: None,
            extra_auth_params: None,
            custom_headers: None,
        };
        let fixture_tokens = OAuthTokens::new(
            "access_token",
            None::<String>,
            chrono::Utc::now() + chrono::Duration::minutes(30),
        );
        let fixture_url_params = HashMap::from([(
            URLParam::from("chatgpt_account_id".to_string()),
            "acct_123".to_string().into(),
        )]);
        let fixture_credential =
            AuthCredential::new_oauth(ProviderId::CODEX, fixture_tokens, fixture_config.clone())
                .url_params(fixture_url_params.clone());

        let actual = refresh_oauth_credential(
            &fixture_credential,
            &fixture_config,
            chrono::Duration::hours(1),
            false,
        )
        .await
        .unwrap();

        let expected = fixture_url_params;
        assert_eq!(actual.url_params, expected);
    }
}
