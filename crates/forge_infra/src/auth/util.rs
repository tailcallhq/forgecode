use std::collections::HashMap;

use chrono::Utc;
use forge_domain::{
    AuthCredential, AuthDetails, OAuthConfig, OAuthTokenResponse, OAuthTokens, ProviderId,
};
use oauth2::basic::BasicClient;
use oauth2::{ClientId, RefreshToken, TokenUrl};

use crate::auth::error::Error;

/// Calculate token expiry with fallback duration
pub(crate) fn calculate_token_expiry(
    expires_in: Option<u64>,
    fallback: chrono::Duration,
) -> chrono::DateTime<chrono::Utc> {
    if let Some(seconds) = expires_in {
        Utc::now() + chrono::Duration::seconds(seconds as i64)
    } else {
        Utc::now() + fallback
    }
}

/// Convert oauth2 TokenResponse into domain OAuthTokenResponse
pub(crate) fn into_domain<T: oauth2::TokenResponse>(token: T) -> OAuthTokenResponse {
    OAuthTokenResponse {
        access_token: token.access_token().secret().to_string(),
        refresh_token: token.refresh_token().map(|t| t.secret().to_string()),
        expires_in: token.expires_in().map(|d| d.as_secs()),
        expires_at: None,
        token_type: "Bearer".to_string(),
        scope: token.scopes().map(|scopes| {
            scopes
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        }),
        id_token: None, // oauth2 crate doesn't provide id_token directly
    }
}

/// Build HTTP client with custom headers
pub(crate) fn build_http_client(
    custom_headers: Option<&HashMap<String, String>>,
) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        // Disable redirects to prevent SSRF vulnerabilities
        .redirect(reqwest::redirect::Policy::none());

    if let Some(headers) = custom_headers {
        let mut header_map = reqwest::header::HeaderMap::new();

        for (key, value) in headers {
            let header_name = reqwest::header::HeaderName::try_from(key.as_str())
                .map_err(|e| anyhow::anyhow!("Invalid header name '{key}': {e}"))?;
            let header_value = value
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid header value for '{key}': {e}"))?;
            header_map.insert(header_name, header_value);
        }

        builder = builder.default_headers(header_map);
    }

    Ok(builder.build()?)
}

/// Build OAuth credential with consistent expiry handling
pub(crate) fn build_oauth_credential(
    provider_id: ProviderId,
    token_response: OAuthTokenResponse,
    config: &OAuthConfig,
    default_expiry: chrono::Duration,
) -> anyhow::Result<AuthCredential> {
    let expires_at = calculate_token_expiry(token_response.expires_in, default_expiry);
    let oauth_tokens = OAuthTokens::new(
        token_response.access_token,
        token_response.refresh_token,
        expires_at,
    );
    Ok(AuthCredential::new_oauth(
        provider_id,
        oauth_tokens,
        config.clone(),
    ))
}

/// Extract OAuth tokens from any credential type
pub(crate) fn extract_oauth_tokens(credential: &AuthCredential) -> anyhow::Result<&OAuthTokens> {
    match &credential.auth_details {
        AuthDetails::OAuth { tokens, .. } => Ok(tokens),
        AuthDetails::OAuthWithApiKey { tokens, .. } => Ok(tokens),
        _ => Err(
            Error::RefreshFailed("Invalid credential type for token extraction".to_string()).into(),
        ),
    }
}

/// Refresh OAuth access token using refresh token
pub(crate) async fn refresh_access_token(
    config: &OAuthConfig,
    refresh_token: &str,
) -> anyhow::Result<OAuthTokenResponse> {
    // Build minimal oauth2 client (just need token endpoint)
    let client = BasicClient::new(ClientId::new(config.client_id.to_string()))
        .set_token_uri(TokenUrl::new(config.token_url.to_string())?);

    // Build HTTP client with custom headers
    let http_client = build_http_client(config.custom_headers.as_ref())?;

    let refresh_token = RefreshToken::new(refresh_token.to_string());

    // Use GitHub-compliant HTTP function to handle non-RFC responses
    let http_fn = |req| github_compliant_http_request(http_client.clone(), req);

    let token_result = client
        .exchange_refresh_token(&refresh_token)
        .request_async(&http_fn)
        .await?;

    Ok(into_domain(token_result))
}

/// GitHub-compliant HTTP request handler that fixes status codes for error
/// responses
pub(crate) async fn github_compliant_http_request(
    client: reqwest::Client,
    request: http::Request<Vec<u8>>,
) -> Result<http::Response<Vec<u8>>, reqwest::Error> {
    // Execute the request
    let mut req_builder = client
        .request(request.method().clone(), request.uri().to_string())
        .body(request.body().clone());

    for (name, value) in request.headers() {
        req_builder = req_builder.header(name.as_str(), value.as_bytes());
    }

    let response = req_builder.send().await?;

    // Get status and body
    let status_code = response.status();
    let headers = response.headers().clone();
    let body = response.bytes().await?;

    // GitHub-specific fix: If status is 200 but body contains "error" field,
    // change status to 400 so oauth2 crate recognizes it as an error response
    let fixed_status = if status_code.is_success() {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) {
            if json.get("error").is_some() {
                // This is actually an error response masquerading as success
                http::StatusCode::BAD_REQUEST
            } else {
                status_code
            }
        } else {
            status_code
        }
    } else {
        status_code
    };

    // Build http::Response with corrected status
    let mut response_builder = http::Response::builder().status(fixed_status);

    // Add headers
    for (name, value) in headers.iter() {
        response_builder = response_builder.header(name, value);
    }

    Ok(response_builder
        .body(body.to_vec())
        .expect("Failed to build HTTP response"))
}

/// Inject custom headers into a header map
pub(crate) fn inject_custom_headers(
    headers: &mut reqwest::header::HeaderMap,
    custom_headers: &Option<HashMap<String, String>>,
) {
    use reqwest::header::{HeaderName, HeaderValue};

    if let Some(custom_headers) = custom_headers {
        for (key, value) in custom_headers {
            if let (Ok(name), Ok(val)) = (HeaderName::try_from(key), HeaderValue::from_str(value)) {
                headers.insert(name, val);
            }
        }
    }
}

/// Parse OAuth error responses during polling
pub(crate) fn handle_oauth_error(error_code: &str) -> Result<(), Error> {
    match error_code {
        "authorization_pending" | "slow_down" => Ok(()),
        "expired_token" => Err(Error::Expired),
        "access_denied" => Err(Error::Denied),
        _ => Err(Error::PollFailed(format!("OAuth error: {error_code}"))),
    }
}

/// Parse token response from JSON.
pub(crate) fn parse_token_response(body: &str) -> Result<OAuthTokenResponse, Error> {
    let token_response: OAuthTokenResponse = serde_json::from_str(body)
        .map_err(|e| Error::PollFailed(format!("Failed to parse token response: {e}")))?;

    if token_response.access_token.trim().is_empty() {
        return Err(Error::PollFailed(
            "Missing access_token in response".to_string(),
        ));
    }

    Ok(token_response)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn test_calculate_token_expiry_with_expires_in() {
        let before = Utc::now();
        let expires_at = calculate_token_expiry(Some(3600), Duration::hours(1));
        let after = Utc::now() + Duration::hours(1);

        assert!(expires_at >= before);
        assert!(expires_at <= after);
    }

    #[test]
    fn test_calculate_token_expiry_with_fallback() {
        let before = Utc::now();
        let expires_at = calculate_token_expiry(None, Duration::days(365));
        let after = Utc::now() + Duration::days(365);

        assert!(expires_at >= before);
        assert!(expires_at <= after);
    }

    #[test]
    fn test_parse_token_response_preserves_id_token() {
        let fixture = r#"{
            "access_token": "test_token",
            "refresh_token": "refresh_token",
            "expires_in": 3600,
            "id_token": "test_id_token"
        }"#;

        let actual = parse_token_response(fixture).unwrap();

        assert_eq!(actual.access_token, "test_token");
        assert_eq!(actual.refresh_token, Some("refresh_token".to_string()));
        assert_eq!(actual.expires_in, Some(3600));
        assert_eq!(actual.id_token, Some("test_id_token".to_string()));
    }

    #[test]
    fn test_handle_oauth_error_retryable() {
        assert!(handle_oauth_error("authorization_pending").is_ok());
        assert!(handle_oauth_error("slow_down").is_ok());
    }

    #[test]
    fn test_handle_oauth_error_terminal() {
        assert!(matches!(
            handle_oauth_error("expired_token"),
            Err(Error::Expired)
        ));
        assert!(matches!(
            handle_oauth_error("access_denied"),
            Err(Error::Denied)
        ));
        assert!(matches!(
            handle_oauth_error("unknown_error"),
            Err(Error::PollFailed(_))
        ));
    }
}
