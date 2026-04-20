use std::collections::HashMap;

use chrono::{DateTime, Utc};
use derive_setters::Setters;
use serde::{Deserialize, Serialize};

use crate::{AccessToken, ApiKey, OAuthConfig, ProviderId, RefreshToken, URLParam, URLParamValue};

/// Strategy for providing API keys to a credential.
///
/// Uses untagged serde representation so that a bare string (legacy format)
/// deserializes as [`StaticKey`](Self::StaticKey), preserving backward
/// compatibility with existing `~/.forge/.credentials.json` files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ApiKeyProvider {
    /// A static, user-supplied API key.
    StaticKey(ApiKey),
    /// A shell command executed via `sh -c` whose trimmed stdout is used as the
    /// API key.  Only the `command` is persisted; `last_key` and `expires_at`
    /// are populated at runtime by executing the command.
    HelperCommand {
        command: String,
        #[serde(skip, default)]
        last_key: ApiKey,
        #[serde(skip, default)]
        expires_at: Option<DateTime<Utc>>,
    },
}

impl ApiKeyProvider {
    /// Returns the current API key value.
    ///
    /// For [`StaticKey`](Self::StaticKey) this is the user-supplied key.  For
    /// [`HelperCommand`](Self::HelperCommand) this is the last key obtained by
    /// executing the command (empty until the first refresh).
    pub fn api_key(&self) -> &ApiKey {
        match self {
            Self::StaticKey(key) => key,
            Self::HelperCommand { last_key, .. } => last_key,
        }
    }

    /// Returns `true` when the key should be refreshed before use.
    ///
    /// Static keys never expire.  Helper-command keys expire based on the
    /// `expires_at` field (populated from the command's TTL/Expires metadata).
    /// When `expires_at` is `None` the key is treated as single-use and
    /// refreshed on every call.
    pub fn needs_refresh(&self, buffer: chrono::Duration) -> bool {
        match self {
            Self::StaticKey(_) => false,
            Self::HelperCommand { expires_at: Some(exp), .. } => Utc::now() + buffer >= *exp,
            Self::HelperCommand { expires_at: None, .. } => true,
        }
    }
}

/// Stored authentication credential for a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Setters)]
pub struct AuthCredential {
    pub id: ProviderId,
    pub auth_details: AuthDetails,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub url_params: HashMap<URLParam, URLParamValue>,
}
impl AuthCredential {
    /// Creates a credential with a static API key.
    pub fn new_api_key(id: ProviderId, api_key: ApiKey) -> Self {
        Self {
            id,
            auth_details: AuthDetails::static_api_key(api_key),
            url_params: HashMap::new(),
        }
    }
    /// Creates a credential with OAuth tokens.
    pub fn new_oauth(id: ProviderId, tokens: OAuthTokens, config: OAuthConfig) -> Self {
        Self {
            id,
            auth_details: AuthDetails::OAuth { tokens, config },
            url_params: HashMap::new(),
        }
    }
    /// Creates a credential with OAuth tokens and an API key.
    pub fn new_oauth_with_api_key(
        id: ProviderId,
        tokens: OAuthTokens,
        api_key: ApiKey,
        config: OAuthConfig,
    ) -> Self {
        Self {
            id,
            auth_details: AuthDetails::OAuthWithApiKey { tokens, api_key, config },
            url_params: HashMap::new(),
        }
    }

    /// Creates a credential with a Google Application Default Credentials token.
    pub fn new_google_adc(id: ProviderId, access_token: ApiKey) -> Self {
        Self {
            id,
            auth_details: AuthDetails::GoogleAdc(access_token),
            url_params: HashMap::new(),
        }
    }

    /// Checks if the credential needs to be refreshed.
    pub fn needs_refresh(&self, buffer: chrono::Duration) -> bool {
        match &self.auth_details {
            AuthDetails::ApiKey(provider) => provider.needs_refresh(buffer),
            // Google ADC tokens are short-lived (1 hour) and should always be checked/refreshed
            AuthDetails::GoogleAdc(_) => true,
            AuthDetails::OAuth { tokens, .. } | AuthDetails::OAuthWithApiKey { tokens, .. } => {
                tokens.needs_refresh(buffer)
            }
        }
    }

    /// Gets the OAuth config if this credential is OAuth-based
    pub fn oauth_config(&self) -> Option<&OAuthConfig> {
        match &self.auth_details {
            AuthDetails::OAuth { config, .. } | AuthDetails::OAuthWithApiKey { config, .. } => {
                Some(config)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthDetails {
    #[serde(alias = "ApiKey")]
    ApiKey(ApiKeyProvider),
    #[serde(alias = "GoogleAdc")]
    GoogleAdc(ApiKey),
    #[serde(alias = "OAuth")]
    OAuth {
        tokens: OAuthTokens,
        config: OAuthConfig,
    },
    #[serde(alias = "OAuthWithApiKey")]
    OAuthWithApiKey {
        tokens: OAuthTokens,
        api_key: ApiKey,
        config: OAuthConfig,
    },
}

impl AuthDetails {
    /// Creates a static API key auth details.
    pub fn static_api_key(key: ApiKey) -> Self {
        Self::ApiKey(ApiKeyProvider::StaticKey(key))
    }

    /// Creates an API key auth details backed by a helper command.
    pub fn api_key_from_helper(
        command: String,
        last_key: ApiKey,
        expires_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self::ApiKey(ApiKeyProvider::HelperCommand { command, last_key, expires_at })
    }

    /// Returns the API key if these auth details contain one.
    pub fn api_key(&self) -> Option<&ApiKey> {
        match self {
            AuthDetails::ApiKey(provider) => Some(provider.api_key()),
            AuthDetails::GoogleAdc(api_key) => Some(api_key),
            AuthDetails::OAuth { .. } => None,
            AuthDetails::OAuthWithApiKey { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: AccessToken,
    pub refresh_token: Option<RefreshToken>,
    pub expires_at: DateTime<Utc>,
}

impl OAuthTokens {
    pub fn new(
        access_token: impl ToString,
        refresh_token: Option<impl ToString>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            access_token: access_token.to_string().into(),
            refresh_token: refresh_token.map(|a| a.to_string().into()),
            expires_at,
        }
    }

    /// Checks if the token is expired or will expire within the given buffer
    /// duration
    pub fn needs_refresh(&self, buffer: chrono::Duration) -> bool {
        let now = Utc::now();
        now + buffer >= self.expires_at
    }

    /// Checks if the token is currently expired
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod api_key_provider {
        use super::*;

        mod static_key {
            use pretty_assertions::assert_eq;

            use super::*;

            #[test]
            fn api_key_returns_the_key() {
                let fixture = ApiKeyProvider::StaticKey(ApiKey::from("sk-test".to_string()));
                let actual = fixture.api_key();
                let expected = &ApiKey::from("sk-test".to_string());
                assert_eq!(actual, expected);
            }

            #[test]
            fn serde_roundtrip() {
                let fixture =
                    ApiKeyProvider::StaticKey(ApiKey::from("sk-test".to_string()));
                let json = serde_json::to_string(&fixture).unwrap();
                let actual: ApiKeyProvider = serde_json::from_str(&json).unwrap();
                assert_eq!(actual, fixture);
            }

            #[test]
            fn serializes_as_bare_string() {
                let fixture =
                    ApiKeyProvider::StaticKey(ApiKey::from("sk-test".to_string()));
                let actual = serde_json::to_string(&fixture).unwrap();
                let expected = r#""sk-test""#;
                assert_eq!(actual, expected);
            }

            #[test]
            fn deserializes_from_bare_string() {
                let actual: ApiKeyProvider = serde_json::from_str(r#""sk-old-key""#).unwrap();
                let expected = ApiKeyProvider::StaticKey(ApiKey::from("sk-old-key".to_string()));
                assert_eq!(actual, expected);
            }
        }

        mod helper_command {
            use pretty_assertions::assert_eq;

            use super::*;

            #[test]
            fn api_key_returns_last_key() {
                let fixture = ApiKeyProvider::HelperCommand {
                    command: "echo key".to_string(),
                    last_key: ApiKey::from("dynamic-key".to_string()),
                    expires_at: None,
                };
                let actual = fixture.api_key();
                let expected = &ApiKey::from("dynamic-key".to_string());
                assert_eq!(actual, expected);
            }

            #[test]
            fn serializes_only_command() {
                let fixture = ApiKeyProvider::HelperCommand {
                    command: "vault read -field=token".to_string(),
                    last_key: ApiKey::from("resolved".to_string()),
                    expires_at: None,
                };
                let actual = serde_json::to_string(&fixture).unwrap();
                let expected = r#"{"command":"vault read -field=token"}"#;
                assert_eq!(actual, expected);
            }

            #[test]
            fn deserializes_with_empty_last_key() {
                let json = r#"{"command":"vault read -field=token"}"#;
                let actual: ApiKeyProvider = serde_json::from_str(json).unwrap();
                let expected = ApiKeyProvider::HelperCommand {
                    command: "vault read -field=token".to_string(),
                    last_key: ApiKey::default(),
                    expires_at: None,
                };
                assert_eq!(actual, expected);
            }

            #[test]
            fn deserialized_needs_refresh() {
                let json = r#"{"command":"echo fresh-key"}"#;
                let fixture: ApiKeyProvider = serde_json::from_str(json).unwrap();
                let actual = fixture.needs_refresh(chrono::Duration::minutes(5));
                assert!(actual);
            }
        }
    }

    mod needs_refresh {
        use super::*;

        mod helper_command {
            use pretty_assertions::assert_eq;

            use super::*;

            #[test]
            fn without_expires_at_returns_true() {
                let fixture = AuthCredential {
                    auth_details: AuthDetails::api_key_from_helper(
                        "echo key".to_string(),
                        ApiKey::from("key".to_string()),
                        None,
                    ),
                    ..AuthCredential::new_api_key(
                        ProviderId::from("test".to_string()),
                        ApiKey::from("key".to_string()),
                    )
                };
                let actual = fixture.needs_refresh(chrono::Duration::minutes(5));
                let expected = true;
                assert_eq!(actual, expected);
            }

            #[test]
            fn with_future_expires_at_returns_false() {
                let fixture = AuthCredential {
                    auth_details: AuthDetails::api_key_from_helper(
                        "echo key".to_string(),
                        ApiKey::from("key".to_string()),
                        Some(Utc::now() + chrono::Duration::hours(1)),
                    ),
                    ..AuthCredential::new_api_key(
                        ProviderId::from("test".to_string()),
                        ApiKey::from("key".to_string()),
                    )
                };
                let actual = fixture.needs_refresh(chrono::Duration::minutes(5));
                let expected = false;
                assert_eq!(actual, expected);
            }

            #[test]
            fn with_past_expires_at_returns_true() {
                let fixture = AuthCredential {
                    auth_details: AuthDetails::api_key_from_helper(
                        "echo key".to_string(),
                        ApiKey::from("key".to_string()),
                        Some(Utc::now() - chrono::Duration::minutes(1)),
                    ),
                    ..AuthCredential::new_api_key(
                        ProviderId::from("test".to_string()),
                        ApiKey::from("key".to_string()),
                    )
                };
                let actual = fixture.needs_refresh(chrono::Duration::minutes(5));
                let expected = true;
                assert_eq!(actual, expected);
            }
        }

        mod static_key {
            use pretty_assertions::assert_eq;

            use super::*;

            #[test]
            fn returns_false() {
                let fixture = AuthCredential::new_api_key(
                    ProviderId::from("test".to_string()),
                    ApiKey::from("key".to_string()),
                );
                let actual = fixture.needs_refresh(chrono::Duration::minutes(5));
                let expected = false;
                assert_eq!(actual, expected);
            }
        }
    }

    mod backward_compat {
        use pretty_assertions::assert_eq;

        use super::*;

        #[test]
        fn legacy_credential_json_deserializes() {
            let fixture = r#"{
                "id": "anthropic",
                "auth_details": {"api_key": "sk-legacy-key"}
            }"#;
            let actual: AuthCredential = serde_json::from_str(fixture).unwrap();
            let expected = AuthCredential::new_api_key(
                ProviderId::from("anthropic".to_string()),
                ApiKey::from("sk-legacy-key".to_string()),
            );
            assert_eq!(actual, expected);
        }

        #[test]
        fn helper_credential_serializes_as_expected() {
            let fixture = vec![AuthCredential {
                id: ProviderId::from("xai".to_string()),
                auth_details: AuthDetails::api_key_from_helper(
                    "printf 'sk-test\\n---\\nTTL: 300'".to_string(),
                    ApiKey::from("sk-test".to_string()),
                    None,
                ),
                url_params: HashMap::new(),
            }];
            let actual = serde_json::to_string_pretty(&fixture).unwrap();
            // command persisted, last_key and expires_at skipped
            assert!(actual.contains(r#""command""#), "should contain command: {actual}");
            assert!(!actual.contains("last_key"), "should NOT contain last_key: {actual}");
            assert!(!actual.contains("expires_at"), "should NOT contain expires_at: {actual}");
        }
    }
}
