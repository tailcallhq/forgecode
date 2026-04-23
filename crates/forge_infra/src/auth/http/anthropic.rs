use std::borrow::Cow;

use forge_app::OAuthHttpProvider;
use forge_domain::{AuthCodeParams, OAuthConfig, OAuthTokenResponse};
use oauth2::PkceCodeChallenge;
use serde::Serialize;
use url::Url;

use crate::auth::util::build_http_client;

/// Anthropic Provider - Non-standard PKCE implementation
/// Quirk: state parameter equals PKCE verifier
#[allow(unused)]
pub struct AnthropicHttpProvider;

#[allow(unused)]
#[derive(Debug, Serialize)]
struct AnthropicTokenRequest {
    /// Authorization code from callback
    code: String,
    /// State parameter (equals PKCE verifier)
    state: String,
    /// Must be "authorization_code"
    grant_type: String,
    /// OAuth client ID
    client_id: String,
    /// Redirect URI (must match authorization request)
    #[serde(skip_serializing_if = "Option::is_none")]
    redirect_uri: Option<String>,
    /// PKCE code verifier
    code_verifier: String,
}

#[async_trait::async_trait]
impl OAuthHttpProvider for AnthropicHttpProvider {
    async fn build_auth_url(&self, config: &OAuthConfig) -> anyhow::Result<AuthCodeParams> {
        // Anthropic quirk: state = verifier
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();

        let mut url = config.auth_url.clone();
        url.query_pairs_mut()
            .append_pair("client_id", &config.client_id)
            .append_pair("response_type", "code")
            .append_pair("scope", &config.scopes.join(" "))
            .append_pair("code_challenge", challenge.as_str())
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", verifier.secret()); // ← Non-standard!

        if let Some(redirect_uri) = &config.redirect_uri {
            url.query_pairs_mut()
                .append_pair("redirect_uri", redirect_uri);
        }

        if let Some(extra_params) = &config.extra_auth_params {
            for (key, value) in extra_params {
                url.query_pairs_mut().append_pair(key, value);
            }
        }

        Ok(AuthCodeParams {
            auth_url: url.to_string(),
            state: verifier.secret().to_string(),
            code_verifier: Some(verifier.secret().to_string()),
        })
    }

    async fn exchange_code(
        &self,
        config: &OAuthConfig,
        code: &str,
        verifier: Option<&str>,
    ) -> anyhow::Result<OAuthTokenResponse> {
        // Anthropic-specific token exchange
        // Supports multiple input formats:
        // 1. Full callback URL: https://platform.claude.com/oauth/code/callback?code=abc&state=def
        // 2. Hash format: code#state
        // 3. Query string: code=abc&state=def
        // 4. Raw code (fallback)
        let trimmed = code.trim();

        let (auth_code, state) = if let Ok(url) = Url::parse(trimmed) {
            // Full URL format - extract code and state from query params
            let auth_code = url
                .query_pairs()
                .find(|(k, _): &(Cow<'_, str>, _)| k == "code")
                .map(|(_, v): (Cow<'_, str>, _)| v.to_string());
            let state = url
                .query_pairs()
                .find(|(k, _): &(Cow<'_, str>, _)| k == "state")
                .map(|(_, v): (Cow<'_, str>, _)| v.to_string());
            (auth_code, state)
        } else if trimmed.contains('#') {
            let parts: Vec<&str> = trimmed.split('#').collect();
            (
                parts.first().map(|s| s.to_string()),
                parts.get(1).map(|s| s.to_string()),
            )
        } else if trimmed.contains('=') {
            // Try parsing as query string
            let params: std::collections::HashMap<String, String> =
                url::form_urlencoded::parse(trimmed.as_bytes())
                    .into_owned()
                    .collect();
            (params.get("code").cloned(), params.get("state").cloned())
        } else {
            (Some(trimmed.to_string()), None)
        };

        let auth_code = auth_code.ok_or_else(|| anyhow::anyhow!("Could not extract authorization code from input"))?;

        let verifier = verifier
            .ok_or_else(|| anyhow::anyhow!("PKCE verifier required for Anthropic OAuth"))?;

        let request_body = AnthropicTokenRequest {
            code: auth_code,
            state: state.unwrap_or_else(|| verifier.to_string()),
            grant_type: "authorization_code".to_string(),
            client_id: config.client_id.to_string(),
            redirect_uri: config.redirect_uri.clone(),
            code_verifier: verifier.to_string(),
        };

        let client = self.build_http_client(config)?;
        let response = client
            .post(config.token_url.as_str())
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Token exchange failed with status {status}: {error_text}");
        }

        Ok(response.json().await?)
    }

    /// Create HTTP client with provider-specific headers/behavior
    fn build_http_client(&self, config: &OAuthConfig) -> anyhow::Result<reqwest::Client> {
        build_http_client(config.custom_headers.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::OAuthConfig;
    use url::Url;

    use super::*;

    fn test_oauth_config() -> OAuthConfig {
        OAuthConfig {
            client_id: "test_client".to_string().into(),
            auth_url: Url::parse("https://example.com/auth").unwrap(),
            token_url: Url::parse("https://example.com/token").unwrap(),
            scopes: vec!["read".to_string(), "write".to_string()],
            redirect_uri: Some("https://example.com/callback".to_string()),
            use_pkce: true,
            token_refresh_url: None,
            extra_auth_params: None,
            custom_headers: None,
        }
    }

    #[tokio::test]
    async fn test_anthropic_provider_state_equals_verifier() {
        let provider = AnthropicHttpProvider;
        let config = test_oauth_config();

        let result = provider.build_auth_url(&config).await.unwrap();

        // Anthropic quirk: state should equal verifier
        assert_eq!(result.state, result.code_verifier.unwrap());
        assert!(result.auth_url.contains("code_challenge_method=S256"));
    }
}
