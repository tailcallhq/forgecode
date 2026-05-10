use serde::{Deserialize, Serialize};

#[derive(
    Clone, Serialize, Deserialize, derive_more::From, derive_more::Deref, PartialEq, Eq, Hash, Debug,
)]
#[serde(transparent)]
pub struct ApiKey(String);

impl std::fmt::Display for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", truncate_key(&self.0))
    }
}

impl AsRef<str> for ApiKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Truncates a key string for display purposes
///
/// If the key length is 20 characters or less, returns it unchanged.
/// Otherwise, shows the first 13 characters and last 4 characters with "..." in
/// between.
///
/// # Arguments
/// * `key` - The key string to truncate
///
/// # Returns
/// * A truncated version of the key for safe display
pub fn truncate_key(key: &str) -> String {
    let char_count = key.chars().count();
    if char_count <= 20 {
        key.to_string()
    } else {
        let prefix: String = key.chars().take(13).collect();
        let suffix: String = key.chars().skip(char_count - 4).collect();
        format!("{prefix}...{suffix}")
    }
}

#[derive(
    Clone, Serialize, Deserialize, derive_more::From, derive_more::Deref, PartialEq, Eq, Debug,
)]
#[serde(transparent)]
pub struct AuthorizationCode(String);

#[derive(
    Clone, Serialize, Deserialize, derive_more::From, derive_more::Deref, PartialEq, Eq, Debug,
)]
#[serde(transparent)]
pub struct DeviceCode(String);

#[derive(
    Clone, Serialize, Deserialize, derive_more::From, derive_more::Deref, PartialEq, Eq, Debug,
)]
#[serde(transparent)]
pub struct PkceVerifier(String);

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    derive_more::Deref,
    Hash,
    derive_more::From,
    derive_more::Display,
)]
#[serde(transparent)]
pub struct URLParam(String);

#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, derive_more::Deref, derive_more::From,
)]
#[serde(transparent)]
pub struct URLParamValue(String);

/// A URL parameter specification with its name and optional preset options.
///
/// When `options` is `Some`, the UI presents a dropdown for selection.
/// When `options` is `None`, the UI presents a free-text input.
/// When `optional` is `true`, the parameter may be left blank and missing
/// values are silently ignored during credential creation and URL rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct URLParamSpec {
    /// The parameter name used as the template variable and credential map key.
    pub name: URLParam,
    /// Optional list of allowed values. When present, the UI renders a
    /// dropdown.
    pub options: Option<Vec<String>>,
    /// Whether this parameter is optional. When `true`, the parameter may be
    /// left blank without causing an error.
    #[serde(default)]
    pub optional: bool,
}

impl URLParamSpec {
    /// Creates a `URLParamSpec` with only a name, rendering as a free-text
    /// input.
    pub fn new(name: impl Into<URLParam>) -> Self {
        Self { name: name.into(), options: None, optional: false }
    }

    /// Creates a `URLParamSpec` with preset options, rendering as a dropdown.
    pub fn with_options(name: impl Into<URLParam>, options: Vec<String>) -> Self {
        Self { name: name.into(), options: Some(options), optional: false }
    }

    /// Creates an optional `URLParamSpec` that may be left blank.
    pub fn optional(name: impl Into<URLParam>) -> Self {
        Self { name: name.into(), options: None, optional: true }
    }
}

impl From<URLParam> for URLParamSpec {
    fn from(name: URLParam) -> Self {
        Self::new(name)
    }
}

impl From<String> for URLParamSpec {
    fn from(name: String) -> Self {
        Self::new(URLParam::from(name))
    }
}

#[derive(
    Clone,
    Serialize,
    Deserialize,
    derive_more::From,
    derive_more::Display,
    derive_more::Deref,
    Debug,
    PartialEq,
    Eq,
)]
#[serde(transparent)]
pub struct UserCode(String);

#[derive(
    Clone, Serialize, Deserialize, derive_more::From, derive_more::Deref, PartialEq, Eq, Debug,
)]
#[serde(transparent)]
pub struct State(String);

#[derive(
    Clone, Serialize, Deserialize, derive_more::From, derive_more::Deref, PartialEq, Eq, Debug,
)]
#[serde(transparent)]
pub struct RefreshToken(String);

#[derive(
    Clone,
    Serialize,
    Deserialize,
    derive_more::From,
    derive_more::Display,
    derive_more::Deref,
    PartialEq,
    Eq,
    Debug,
)]
#[serde(transparent)]
pub struct AccessToken(String);

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_truncate_key_short_key() {
        let fixture = "sk-abc123";
        let actual = truncate_key(fixture);
        let expected = "sk-abc123";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_truncate_key_long_ascii_key() {
        let fixture = "sk-1234567890abcdefghijklmnop";
        let actual = truncate_key(fixture);
        let expected = "sk-1234567890...mnop";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_truncate_key_multibyte_chars_no_panic() {
        // Keys with multi-byte UTF-8 characters should not panic
        let fixture = "sk-12345678→→→→→→→→→→abcd";
        let actual = truncate_key(fixture);
        let expected = "sk-12345678→→...abcd";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_truncate_key_emoji_chars_no_panic() {
        // Keys with 4-byte emoji characters should not panic
        // 25 chars: a(13) + 🔑(8) + b(4) = 25
        let fixture = "aaaaaaaaaaaaa🔑🔑🔑🔑🔑🔑🔑🔑bbbb";
        let actual = truncate_key(fixture);
        let expected = "aaaaaaaaaaaaa...bbbb";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_truncate_key_exactly_20_chars() {
        let fixture = "12345678901234567890";
        let actual = truncate_key(fixture);
        let expected = "12345678901234567890";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_truncate_key_21_chars() {
        let fixture = "123456789012345678901";
        let actual = truncate_key(fixture);
        let expected = "1234567890123...8901";
        assert_eq!(actual, expected);
    }
}
