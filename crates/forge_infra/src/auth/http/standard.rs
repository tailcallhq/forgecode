use forge_app::OAuthHttpProvider;
use forge_domain::{AuthCodeParams, OAuthConfig, OAuthTokenResponse};
use oauth2::{CsrfToken, PkceCodeChallenge, Scope};
use serde::Serialize;

use crate::auth::util::*;

/// Standard RFC-compliant OAuth provider
pub struct StandardHttpProvider;

#[derive(Debug, Serialize)]
struct StandardTokenRequest<'a> {
    grant_type: &'static str,
    code: &'a str,
    client_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    redirect_uri: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code_verifier: Option<&'a str>,
}

#[async_trait::async_trait]
impl OAuthHttpProvider for StandardHttpProvider {
    async fn build_auth_url(&self, config: &OAuthConfig) -> anyhow::Result<AuthCodeParams> {
        // Use oauth2 library - standard flow
        use oauth2::{AuthUrl, ClientId, TokenUrl};

        let mut client =
            oauth2::basic::BasicClient::new(ClientId::new(config.client_id.to_string()))
                .set_auth_uri(AuthUrl::new(config.auth_url.to_string())?)
                .set_token_uri(TokenUrl::new(config.token_url.to_string())?);

        if let Some(redirect_uri) = &config.redirect_uri {
            client = client.set_redirect_uri(oauth2::RedirectUrl::new(redirect_uri.clone())?);
        }

        let mut request = client.authorize_url(CsrfToken::new_random);

        for scope in &config.scopes {
            request = request.add_scope(Scope::new(scope.clone()));
        }

        if let Some(extra_params) = &config.extra_auth_params {
            for (key, value) in extra_params {
                request = request.add_extra_param(key, value);
            }
        }

        let (auth_url, csrf_state, pkce_verifier) = if config.use_pkce {
            let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
            let (url, state) = request.set_pkce_challenge(challenge).url();
            (url, state, Some(verifier))
        } else {
            let (url, state) = request.url();
            (url, state, None)
        };

        Ok(AuthCodeParams {
            auth_url: auth_url.to_string(),
            state: csrf_state.secret().to_string(),
            code_verifier: pkce_verifier.map(|v| v.secret().to_string()),
        })
    }

    async fn exchange_code(
        &self,
        config: &OAuthConfig,
        code: &str,
        verifier: Option<&str>,
    ) -> anyhow::Result<OAuthTokenResponse> {
        let http_client = self.build_http_client(config)?;
        let request_body = StandardTokenRequest {
            grant_type: "authorization_code",
            code,
            client_id: config.client_id.as_ref(),
            redirect_uri: config.redirect_uri.as_deref(),
            code_verifier: verifier,
        };

        let response = http_client
            .post(config.token_url.as_str())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(serde_urlencoded::to_string(&request_body)?)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("OAuth token exchange failed ({status}): {body}");
        }

        // Parse the raw token payload so provider-specific fields like
        // `id_token` are preserved instead of being dropped by generic helpers.
        Ok(parse_token_response(&body)?)
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
    async fn test_standard_provider_build_auth_url() {
        let provider = StandardHttpProvider;
        let config = test_oauth_config();

        let result = provider.build_auth_url(&config).await.unwrap();

        assert!(result.auth_url.contains("client_id=test_client"));
        assert!(result.auth_url.contains("response_type=code"));
        assert!(result.code_verifier.is_some());
        assert_ne!(&result.state, result.code_verifier.as_ref().unwrap());
    }
}
