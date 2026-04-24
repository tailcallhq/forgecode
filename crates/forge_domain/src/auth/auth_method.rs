use serde::{Deserialize, Serialize};

use super::OAuthConfig;

/// Authentication method configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    ApiKey,
    #[serde(rename = "oauth_device")]
    OAuthDevice(OAuthConfig),
    #[serde(rename = "oauth_code")]
    OAuthCode(OAuthConfig),
    #[serde(rename = "google_adc")]
    GoogleAdc,
    #[serde(rename = "aws_profile")]
    AwsProfile,
    #[serde(rename = "codex_device")]
    CodexDevice(OAuthConfig),
}

impl AuthMethod {
    pub fn oauth_device(config: OAuthConfig) -> Self {
        Self::OAuthDevice(config)
    }

    pub fn oauth_code(config: OAuthConfig) -> Self {
        Self::OAuthCode(config)
    }

    pub fn google_adc() -> Self {
        Self::GoogleAdc
    }

    /// Creates a Codex device auth method
    pub fn codex_device(config: OAuthConfig) -> Self {
        Self::CodexDevice(config)
    }

    pub fn oauth_config(&self) -> Option<&OAuthConfig> {
        match self {
            Self::OAuthDevice(config) | Self::OAuthCode(config) | Self::CodexDevice(config) => {
                Some(config)
            }
            Self::ApiKey | Self::GoogleAdc | Self::AwsProfile => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use url::Url;

    use super::*;

    fn oauth_config_fixture() -> OAuthConfig {
        OAuthConfig {
            auth_url: Url::parse("https://auth.openai.com/api/accounts/deviceauth/usercode")
                .unwrap(),
            token_url: Url::parse("https://auth.openai.com/oauth/token").unwrap(),
            client_id: "app_test".to_string().into(),
            scopes: vec!["openid".to_string()],
            redirect_uri: None,
            use_pkce: false,
            token_refresh_url: None,
            custom_headers: None,
            extra_auth_params: None,
        }
    }

    #[test]
    fn test_codex_device_constructor() {
        let config = oauth_config_fixture();
        let actual = AuthMethod::codex_device(config.clone());
        let expected = AuthMethod::CodexDevice(config);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_codex_device_oauth_config_returns_some() {
        let config = oauth_config_fixture();
        let method = AuthMethod::CodexDevice(config.clone());
        let actual = method.oauth_config();
        assert_eq!(actual, Some(&config));
    }

    #[test]
    fn test_codex_device_serde_roundtrip() {
        let config = oauth_config_fixture();
        let method = AuthMethod::CodexDevice(config);
        let serialized = serde_json::to_string(&method).unwrap();
        let actual: AuthMethod = serde_json::from_str(&serialized).unwrap();
        assert_eq!(actual, method);
    }

    #[test]
    fn test_codex_device_deserializes_from_json() {
        let json = serde_json::json!({
            "codex_device": {
                "auth_url": "https://auth.openai.com/api/accounts/deviceauth/usercode",
                "token_url": "https://auth.openai.com/oauth/token",
                "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
                "scopes": ["openid", "profile"],
                "use_pkce": false
            }
        });
        let actual: AuthMethod = serde_json::from_value(json).unwrap();
        assert!(matches!(actual, AuthMethod::CodexDevice(_)));
        assert!(actual.oauth_config().is_some());
        assert_eq!(
            actual.oauth_config().unwrap().client_id.as_str(),
            "app_EMoamEEZ73f0CkXaXp7hrann"
        );
    }
}
