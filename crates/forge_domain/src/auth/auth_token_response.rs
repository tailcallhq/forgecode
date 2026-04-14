use serde::{Deserialize, Serialize};

/// OAuth token response structure
#[derive(Debug, Clone, Serialize, Deserialize)]
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

fn default_token_type() -> String {
    "Bearer".to_string()
}
