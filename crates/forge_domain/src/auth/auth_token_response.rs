use serde::{Deserialize, Serialize};

/// OAuth token response structure
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthTokenResponse {
    /// Access token for API requests
    #[serde(alias = "token")]
    pub access_token: String,

    /// Refresh token for obtaining new access tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// Seconds until access token expires
    #[serde(skip_serializing_if = "Option::is_none", alias = "refresh_in")]
    pub expires_in: Option<u64>,

    /// Unix timestamp when token expires (GitHub Copilot pattern)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,

    /// Token type (usually "Bearer")
    #[serde(default = "default_token_type")]
    pub token_type: String,

    /// OAuth scopes granted
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,

    /// ID token containing user identity claims (OpenID Connect)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
}

impl std::fmt::Debug for OAuthTokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokenResponse")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_in", &self.expires_in)
            .field("expires_at", &self.expires_at)
            .field("token_type", &self.token_type)
            .field("scope", &self.scope)
            .field("id_token", &self.id_token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oauth_token_response_debug_redacts_secrets() {
        let response = OAuthTokenResponse {
            access_token: "super_secret_access_token_xyz".to_string(),
            refresh_token: Some("super_secret_refresh_token_xyz".to_string()),
            expires_in: Some(3600),
            expires_at: None,
            token_type: "Bearer".to_string(),
            scope: Some("read write".to_string()),
            id_token: Some("super_secret_id_token_xyz".to_string()),
        };
        let debug = format!("{:?}", response);
        assert!(
            !debug.contains("super_secret_access_token_xyz"),
            "access_token must be redacted in Debug"
        );
        assert!(
            !debug.contains("super_secret_refresh_token_xyz"),
            "refresh_token must be redacted in Debug"
        );
        assert!(
            !debug.contains("super_secret_id_token_xyz"),
            "id_token must be redacted in Debug"
        );
        assert!(
            debug.contains("<redacted>"),
            "Debug output must contain <redacted>"
        );
        // Non-secret fields should remain visible
        assert!(
            debug.contains("Bearer"),
            "token_type should remain visible in Debug"
        );
        assert!(
            debug.contains("3600"),
            "expires_in should remain visible in Debug"
        );
    }
}
