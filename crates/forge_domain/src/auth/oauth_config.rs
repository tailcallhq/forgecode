use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use url::Url;

#[derive(
    Clone, Serialize, Deserialize, derive_more::From, derive_more::Deref, PartialEq, Eq, Debug,
)]
#[serde(transparent)]
pub struct ClientId(String);

/// OAuth configuration for authentication flows
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub auth_url: Url,
    pub token_url: Url,
    pub client_id: ClientId,
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub use_pkce: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_refresh_url: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_headers: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_auth_params: Option<HashMap<String, String>>,
}

/// An unsafe or unsupported Copilot authentication destination.
#[derive(Debug, thiserror::Error)]
#[error("Invalid Copilot destination: use github.com or <enterprise>.ghe.com over HTTPS")]
pub struct CopilotHostError;

impl OAuthConfig {
    /// Selects the GitHub host for Copilot device authentication.
    ///
    /// # Arguments
    /// * `host` - A bare github.com or single-label enterprise.ghe.com hostname.
    ///
    /// # Errors
    /// Rejects URLs, ports, credentials and unsupported hostnames.
    pub fn with_copilot_host(mut self, host: &str) -> Result<Self, CopilotHostError> {
        let host = host.to_ascii_lowercase();
        let enterprise = host.strip_suffix(".ghe.com");
        if host != "github.com" && !enterprise.is_some_and(valid_host_label) {
            return Err(CopilotHostError);
        }
        let api_host = if host == "github.com" {
            "api.github.com".to_string()
        } else {
            format!("api.{host}")
        };
        self.auth_url = Url::parse(&format!("https://{host}/login/device/code"))
            .map_err(|_| CopilotHostError)?;
        self.token_url = Url::parse(&format!("https://{host}/login/oauth/access_token"))
            .map_err(|_| CopilotHostError)?;
        self.token_refresh_url = Some(
            Url::parse(&format!("https://{api_host}/copilot_internal/v2/token"))
                .map_err(|_| CopilotHostError)?,
        );
        Ok(self)
    }

    /// Validates persisted Copilot OAuth destinations before sending credentials.
    ///
    /// # Errors
    /// Rejects any endpoint not belonging to the selected GitHub host.
    pub fn validate_copilot(&self) -> Result<(), CopilotHostError> {
        let host = self.auth_url.host_str().ok_or(CopilotHostError)?;
        let expected = self.clone().with_copilot_host(host)?;
        if self.auth_url != expected.auth_url
            || self.token_url != expected.token_url
            || self.token_refresh_url != expected.token_refresh_url
        {
            return Err(CopilotHostError);
        }
        Ok(())
    }

    /// Validates the browser device verification URL against the selected host.
    ///
    /// # Arguments
    /// * `url` - The verification URL returned by the device authorization service.
    ///
    /// # Errors
    /// Rejects non-HTTPS, credential-bearing or cross-host verification URLs.
    pub fn validate_copilot_verification(&self, url: &Url) -> Result<(), CopilotHostError> {
        self.validate_copilot()?;
        if !safe_https_origin(url) || url.host_str() != self.auth_url.host_str() {
            return Err(CopilotHostError);
        }
        Ok(())
    }

    /// Validates the Copilot API base returned by the authenticated token service.
    /// Enterprise credentials must stay within their own enterprise's domain.
    ///
    /// # Arguments
    /// * `url` - The API base from the token response or saved credential.
    ///
    /// # Errors
    /// Rejects insecure, cross-enterprise or non-Copilot public endpoints.
    pub fn validate_copilot_api(&self, url: &Url) -> Result<(), CopilotHostError> {
        self.validate_copilot()?;
        let host = url.host_str().ok_or(CopilotHostError)?;
        let auth_host = self.auth_url.host_str().ok_or(CopilotHostError)?;
        let suffix = if auth_host == "github.com" {
            ".githubcopilot.com".to_string()
        } else {
            format!(".{auth_host}")
        };
        if !safe_https_origin(url)
            || !host.ends_with(&suffix)
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(CopilotHostError);
        }
        Ok(())
    }
}

fn valid_host_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

fn safe_https_origin(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn fixture() -> OAuthConfig {
        serde_json::from_value(serde_json::json!({
            "auth_url": "https://github.com/login/device/code",
            "token_url": "https://github.com/login/oauth/access_token",
            "token_refresh_url": "https://api.github.com/copilot_internal/v2/token",
            "client_id": "test-client", "scopes": ["read:user"]
        }))
        .unwrap()
    }

    #[test]
    fn copilot_default_is_unchanged() {
        let fixture = fixture();
        let actual = fixture.clone().with_copilot_host("github.com").unwrap();
        let expected = fixture;
        assert_eq!(actual, expected);
    }

    #[test]
    fn copilot_enterprise_destinations() {
        let fixture = fixture();
        let actual = fixture.with_copilot_host("Octo-Corp.ghe.com").unwrap();
        let expected = (
            "https://octo-corp.ghe.com/login/device/code",
            "https://octo-corp.ghe.com/login/oauth/access_token",
            "https://api.octo-corp.ghe.com/copilot_internal/v2/token",
        );
        assert_eq!(
            (
                actual.auth_url.as_str(),
                actual.token_url.as_str(),
                actual.token_refresh_url.as_ref().unwrap().as_str()
            ),
            expected
        );
        actual.validate_copilot().unwrap();
    }

    #[test]
    fn copilot_invalid_hosts() {
        let fixture = fixture();
        for host in [
            "",
            "ghe.com",
            ".ghe.com",
            "a.b.ghe.com",
            "-a.ghe.com",
            "a-.ghe.com",
            "a_ghe.ghe.com",
            "evil.com",
            "github.com.evil.com",
            "company.ghe.com.evil.com",
            "https://company.ghe.com",
            "user@company.ghe.com",
            "company.ghe.com:443",
            "company.ghe.com/",
            "company.ghe.com?x",
            "company.ghe.com#x",
            "company.ghe.com\\evil",
            " company.ghe.com",
            "cömpany.ghe.com",
        ] {
            let actual = fixture.clone().with_copilot_host(host);
            assert!(actual.is_err(), "accepted {host}");
        }
    }

    #[test]
    fn copilot_rejects_tampered_token_destinations() {
        for destination in [
            "http://api.company.ghe.com/copilot_internal/v2/token",
            "https://api.other.ghe.com/copilot_internal/v2/token",
            "https://evil.com/token",
            "https://user@api.company.ghe.com/copilot_internal/v2/token",
        ] {
            let mut fixture = fixture().with_copilot_host("company.ghe.com").unwrap();
            fixture.token_refresh_url = Some(Url::parse(destination).unwrap());
            let actual = fixture.validate_copilot();
            assert!(actual.is_err());
        }
    }

    #[test]
    fn copilot_enterprise_api_stays_within_enterprise() {
        let fixture = fixture().with_copilot_host("company.ghe.com").unwrap();
        fixture
            .validate_copilot_api(&Url::parse("https://copilot-api.company.ghe.com").unwrap())
            .unwrap();
        for destination in [
            "https://api.githubcopilot.com",
            "https://copilot-api.other.ghe.com",
            "https://company.ghe.com.evil.com",
            "http://copilot-api.company.ghe.com",
            "https://user@copilot-api.company.ghe.com",
            "https://copilot-api.company.ghe.com:8443",
            "https://copilot-api.company.ghe.com/path",
            "https://copilot-api.company.ghe.com/?key=x",
        ] {
            let actual = fixture.validate_copilot_api(&Url::parse(destination).unwrap());
            assert!(actual.is_err(), "accepted {destination}");
        }
    }

    #[test]
    fn copilot_verification_rejects_cross_host() {
        let fixture = fixture().with_copilot_host("company.ghe.com").unwrap();
        fixture
            .validate_copilot_verification(
                &Url::parse("https://company.ghe.com/login/device").unwrap(),
            )
            .unwrap();
        let actual = fixture
            .validate_copilot_verification(&Url::parse("https://github.com/login/device").unwrap());
        assert!(actual.is_err());
    }
}
