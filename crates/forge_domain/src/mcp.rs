//!
//! Follows the design specifications of Claude's [.mcp.json](https://docs.anthropic.com/en/docs/claude-code/tutorials#set-up-model-context-protocol-mcp)

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Deref;
use std::path::Path;

use derive_more::{Deref, Display, From};
use derive_setters::Setters;
use merge::Merge;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    Local,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Hash)]
#[serde(untagged)]
pub enum McpServerConfig {
    Stdio(McpStdioServer),
    Http(McpHttpServer),
}

impl McpServerConfig {
    /// Create a new stdio-based MCP server
    pub fn new_stdio(
        command: impl Into<String>,
        args: Vec<String>,
        env: Option<BTreeMap<String, String>>,
    ) -> Self {
        Self::Stdio(McpStdioServer {
            command: command.into(),
            args,
            env: env.unwrap_or_default(),
            timeout: None,
            disable: false,
        })
    }

    /// Create a new HTTP-based MCP server (auto-detects transport type)
    pub fn new_http(url: impl Into<String>) -> Self {
        Self::Http(McpHttpServer {
            url: url.into(),
            headers: BTreeMap::new(),
            timeout: None,
            disable: false,
            oauth: McpOAuthSetting::AutoDetect,
        })
    }

    pub fn is_disabled(&self) -> bool {
        match self {
            McpServerConfig::Stdio(v) => v.disable,
            McpServerConfig::Http(v) => v.disable,
        }
    }

    /// Returns the type of MCP server as a string ("STDIO" or "HTTP")
    pub fn server_type(&self) -> &'static str {
        match self {
            McpServerConfig::Stdio(_) => "STDIO",
            McpServerConfig::Http(_) => "HTTP",
        }
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, Setters, PartialEq, Hash)]
#[setters(strip_option, into)]
pub struct McpStdioServer {
    /// Command to execute for starting this MCP server
    #[serde(skip_serializing_if = "String::is_empty")]
    pub command: String,

    /// Arguments to pass to the command
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,

    /// Environment variables to pass to the command
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,

    /// Timeout in seconds for tool calls to this MCP server
    /// If not specified, uses the default FORGE_MCP_TIMEOUT or 300 seconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// Disable it temporarily without having to
    /// remove it from the config.
    #[serde(default)]
    pub disable: bool,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Hash)]
pub struct McpHttpServer {
    /// Url of the MCP server (auto-detects HTTP vs SSE transport)
    #[serde(skip_serializing_if = "String::is_empty", alias = "serverUrl")]
    pub url: String,

    /// Optional headers for HTTP requests
    /// Supports mustache templates for environment variables: {{.env.VAR_NAME}}
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,

    /// Timeout in seconds for HTTP requests to this MCP server
    /// If not specified, uses the default FORGE_MCP_TIMEOUT or 300 seconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// Disable it temporarily without having to
    /// remove it from the config.
    #[serde(default)]
    pub disable: bool,

    /// OAuth 2.0 configuration for MCP server authentication.
    /// Supports three formats:
    /// - Absent/null: OAuth auto-detection via server 401 response
    /// - `false`: Explicitly disable OAuth (use API key/headers instead)
    /// - `{ ... }`: Explicit OAuth configuration (client_id, scopes, etc.)
    #[serde(
        default,
        skip_serializing_if = "McpOAuthSetting::is_default",
        deserialize_with = "McpOAuthSetting::deserialize_flexible",
        serialize_with = "McpOAuthSetting::serialize_flexible"
    )]
    pub oauth: McpOAuthSetting,
}

impl McpHttpServer {
    /// Returns true if OAuth is explicitly disabled for this server.
    pub fn is_oauth_disabled(&self) -> bool {
        matches!(self.oauth, McpOAuthSetting::Disabled)
    }

    /// Returns the OAuth config if OAuth is explicitly configured.
    pub fn oauth_config(&self) -> Option<&McpOAuthConfig> {
        match &self.oauth {
            McpOAuthSetting::Configured(config) => Some(config),
            _ => None,
        }
    }
}

/// Represents the OAuth setting for an MCP server.
/// Supports three states: auto-detect (default), explicitly disabled, or
/// explicitly configured.
#[derive(Debug, Clone, PartialEq, Hash, Default)]
pub enum McpOAuthSetting {
    /// No explicit OAuth config - auto-detect via server 401 response
    #[default]
    AutoDetect,
    /// OAuth explicitly disabled (`oauth: false`)
    Disabled,
    /// OAuth explicitly configured with parameters
    Configured(McpOAuthConfig),
}

impl McpOAuthSetting {
    /// Returns true if the setting is the default (AutoDetect).
    pub fn is_default(&self) -> bool {
        matches!(self, Self::AutoDetect)
    }

    /// Custom deserializer that accepts:
    /// - boolean `false` -> Disabled
    /// - boolean `true` -> AutoDetect
    /// - null/absent -> AutoDetect
    /// - object `{ ... }` -> Configured(McpOAuthConfig)
    fn deserialize_flexible<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        struct McpOAuthSettingVisitor;

        impl<'de> de::Visitor<'de> for McpOAuthSettingVisitor {
            type Value = McpOAuthSetting;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a boolean or an OAuth config object")
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                if v {
                    Ok(McpOAuthSetting::AutoDetect)
                } else {
                    Ok(McpOAuthSetting::Disabled)
                }
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(McpOAuthSetting::AutoDetect)
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(McpOAuthSetting::AutoDetect)
            }

            fn visit_map<M: de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                let config =
                    McpOAuthConfig::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(McpOAuthSetting::Configured(config))
            }
        }

        deserializer.deserialize_any(McpOAuthSettingVisitor)
    }

    /// Custom serializer:
    /// - AutoDetect -> skip (handled by skip_serializing_if)
    /// - Disabled -> `false`
    /// - Configured -> serialize the config object
    fn serialize_flexible<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::AutoDetect => serializer.serialize_none(),
            Self::Disabled => serializer.serialize_bool(false),
            Self::Configured(config) => config.serialize(serializer),
        }
    }
}

/// MCP OAuth 2.0 configuration.
/// Supports automatic OAuth configuration discovery from server metadata.
/// When auth_url/token_url are not provided, Forge will automatically
/// discover them using RFC 8414 OAuth 2.0 Authorization Server Metadata.
#[derive(Default, Debug, Clone, Serialize, Deserialize, Setters, PartialEq, Hash)]
#[setters(strip_option, into)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthConfig {
    /// Pre-registered OAuth client ID (optional for dynamic registration).
    /// If not provided, dynamic client registration will be attempted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,

    /// Client secret for confidential clients.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,

    /// OAuth scopes to request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,

    /// Authorization endpoint URL.
    /// If not provided, discovered automatically from server metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,

    /// Token endpoint URL.
    /// If not provided, discovered automatically from server metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,

    /// Redirect URI for OAuth callback.
    /// Defaults to http://127.0.0.1:8765/callback.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
}

#[derive(
    Clone, Display, Serialize, Deserialize, Debug, PartialEq, Hash, Eq, From, PartialOrd, Ord, Deref,
)]
pub struct ServerName(String);

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Hash, Merge)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConfig {
    #[merge(strategy = std::collections::BTreeMap::extend)]
    #[serde(default)]
    pub mcp_servers: BTreeMap<ServerName, McpServerConfig>,
}

impl Deref for McpConfig {
    type Target = BTreeMap<ServerName, McpServerConfig>;

    fn deref(&self) -> &Self::Target {
        &self.mcp_servers
    }
}

impl From<BTreeMap<ServerName, McpServerConfig>> for McpConfig {
    fn from(mcp_servers: BTreeMap<ServerName, McpServerConfig>) -> Self {
        Self { mcp_servers }
    }
}

impl McpConfig {
    /// Compute a deterministic u64 identifier for this config.
    ///
    /// Uses FNV-64 (a non-cryptographic but stable, seed-free hasher) so the
    /// same config always produces the same key across process restarts.
    /// This is required for persisted trust-store lookups: `DefaultHasher`
    /// uses a random seed per-process and would produce a different value on
    /// every restart, causing trust decisions to be ignored.
    /// `BTreeMap` ensures consistent field ordering regardless of insertion
    /// order.
    pub fn cache_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = fnv_rs::Fnv64::default();
        Hash::hash(self, &mut hasher);
        hasher.finish()
    }
}

/// The trust status of a single MCP config file (identified by path + content
/// hash).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTrustStatus {
    /// The config has been explicitly accepted by the user.
    Trusted,
    /// The config has been explicitly rejected by the user.
    Rejected,
    /// The config has not yet been decided by the user.
    Unknown,
}

/// A persisted trust decision: stores the user's choice together with the
/// content hash it was made against so that any modification to the file
/// invalidates the decision.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct McpTrustEntry {
    hash: u64,
    decision: McpTrustDecision,
}

/// The two terminal (persisted) states for a trust decision.
///
/// Distinct from `McpTrustStatus` so that the on-disk representation cannot
/// encode the `Unknown` variant (which simply means "no entry").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum McpTrustDecision {
    Trusted,
    Rejected,
}

impl From<McpTrustDecision> for McpTrustStatus {
    fn from(decision: McpTrustDecision) -> Self {
        match decision {
            McpTrustDecision::Trusted => McpTrustStatus::Trusted,
            McpTrustDecision::Rejected => McpTrustStatus::Rejected,
        }
    }
}

/// Persists accepted and rejected MCP config hashes across restarts. A path
/// maps to its content hash so that any modification to the file revokes the
/// stored decision and triggers a new prompt.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct McpTrustStore {
    #[serde(default)]
    entries: BTreeMap<String, McpTrustEntry>,
}

impl McpTrustStore {
    /// Derives the JSON-map key for a path. Returns a borrowed `&str` when
    /// the path is already valid UTF-8, allocating only for paths containing
    /// non-UTF-8 bytes.
    fn key(path: &Path) -> Cow<'_, str> {
        path.to_string_lossy()
    }

    /// Returns the trust status for the given path+hash pair.
    ///
    /// Returns `Unknown` both when no decision has been persisted and when a
    /// decision exists but was made against a different content hash (i.e.
    /// the file has been modified since).
    pub fn get_status(&self, path: &Path, content_hash: u64) -> McpTrustStatus {
        match self.entries.get(Self::key(path).as_ref()) {
            Some(entry) if entry.hash == content_hash => entry.decision.into(),
            _ => McpTrustStatus::Unknown,
        }
    }

    /// Records an accepted trust decision for the given path and content hash.
    pub fn trust(&mut self, path: &Path, content_hash: u64) {
        self.insert(path, content_hash, McpTrustDecision::Trusted);
    }

    /// Records a rejected trust decision for the given path and content hash.
    pub fn reject(&mut self, path: &Path, content_hash: u64) {
        self.insert(path, content_hash, McpTrustDecision::Rejected);
    }

    /// Clears any trust decision (accepted or rejected) for the given path.
    pub fn clear(&mut self, path: &Path) {
        self.entries.remove(Self::key(path).as_ref());
    }

    fn insert(&mut self, path: &Path, hash: u64, decision: McpTrustDecision) {
        self.entries.insert(
            Self::key(path).into_owned(),
            McpTrustEntry { hash, decision },
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_mcp_config_hash_consistency() {
        use pretty_assertions::assert_eq;

        // Create two identical configs
        let fixture1 = McpConfig {
            mcp_servers: BTreeMap::from([
                (
                    "server1".to_string().into(),
                    McpServerConfig::new_http("http://localhost:3000"),
                ),
                (
                    "server2".to_string().into(),
                    McpServerConfig::new_stdio("node", vec![], None),
                ),
            ]),
        };

        let fixture2 = McpConfig {
            mcp_servers: BTreeMap::from([
                (
                    "server1".to_string().into(),
                    McpServerConfig::new_http("http://localhost:3000"),
                ),
                (
                    "server2".to_string().into(),
                    McpServerConfig::new_stdio("node", vec![], None),
                ),
            ]),
        };

        // Hashes should be identical
        let actual = fixture1.cache_key();
        let expected = fixture2.cache_key();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_mcp_config_hash_different_configs() {
        use pretty_assertions::assert_ne;

        // Create two different configs
        let fixture1 = McpConfig {
            mcp_servers: BTreeMap::from([(
                "server1".to_string().into(),
                McpServerConfig::new_http("http://localhost:3000"),
            )]),
        };

        let fixture2 = McpConfig {
            mcp_servers: BTreeMap::from([(
                "server1".to_string().into(),
                McpServerConfig::new_http("http://localhost:3001"),
            )]),
        };

        // Hashes should be different
        let actual = fixture1.cache_key();
        let expected = fixture2.cache_key();
        assert_ne!(actual, expected);
    }

    #[test]
    fn test_mcp_config_hash_insertion_order_independent() {
        use pretty_assertions::assert_eq;

        // Create config with servers in one order
        let fixture1 = McpConfig {
            mcp_servers: BTreeMap::from([
                (
                    "a_server".to_string().into(),
                    McpServerConfig::new_http("http://a"),
                ),
                (
                    "z_server".to_string().into(),
                    McpServerConfig::new_http("http://z"),
                ),
            ]),
        };

        // Create config with servers in different order (BTreeMap sorts by key)
        let fixture2 = McpConfig {
            mcp_servers: BTreeMap::from([
                (
                    "z_server".to_string().into(),
                    McpServerConfig::new_http("http://z"),
                ),
                (
                    "a_server".to_string().into(),
                    McpServerConfig::new_http("http://a"),
                ),
            ]),
        };

        // Hashes should be identical because BTreeMap maintains sorted order
        let actual = fixture1.cache_key();
        let expected = fixture2.cache_key();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_mcp_server_config_disabled() {
        let server = McpStdioServer { disable: true, ..Default::default() };

        let config = McpServerConfig::Stdio(server);
        assert!(config.is_disabled());

        let sse_server = McpHttpServer { disable: false, ..Default::default() };

        let config = McpServerConfig::Http(sse_server);
        assert!(!config.is_disabled());
    }

    #[test]
    fn test_mcp_config_deserialization_valid() {
        use pretty_assertions::assert_eq;

        let json = r#"{
            "mcpServers": {
                "test_server": {
                    "command": "node",
                    "args": ["server.js"]
                }
            }
        }"#;

        let actual: McpConfig = serde_json::from_str(json).unwrap();
        let expected = McpConfig {
            mcp_servers: BTreeMap::from([(
                "test_server".to_string().into(),
                McpServerConfig::new_stdio("node", vec!["server.js".to_string()], None),
            )]),
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_mcp_config_deserialization_empty_object() {
        let json = "{}";
        let result = serde_json::from_str::<McpConfig>(json);

        assert!(result.is_ok());
    }

    #[test]
    fn test_mcp_config_deserialization_wrong_field_name() {
        let json = r#"{"servers": {"test": {}}}"#;
        let result = serde_json::from_str::<McpConfig>(json);

        assert!(result.is_err());
    }

    #[test]
    fn test_mcp_config_deserialization_null_mcp_servers() {
        let json = r#"{"mcpServers": null}"#;
        let result = serde_json::from_str::<McpConfig>(json);

        assert!(result.is_err());
    }

    #[test]
    fn test_http_server_with_headers() {
        use pretty_assertions::assert_eq;

        let json = r#"{
            "mcpServers": {
                "github": {
                    "url": "https://api.githubcopilot.com/mcp/",
                    "headers": {
                        "Authorization": "Bearer test_token",
                        "Content-Type": "application/json"
                    }
                }
            }
        }"#;

        let actual: McpConfig = serde_json::from_str(json).unwrap();

        match actual.mcp_servers.get(&"github".to_string().into()) {
            Some(McpServerConfig::Http(server)) => {
                assert_eq!(server.url, "https://api.githubcopilot.com/mcp/");
                assert_eq!(server.headers.len(), 2);
                assert_eq!(
                    server.headers.get("Authorization"),
                    Some(&"Bearer test_token".to_string())
                );
            }
            _ => panic!("Expected Http variant"),
        }
    }

    #[test]
    fn test_http_server_with_timeout() {
        use pretty_assertions::assert_eq;

        let json = r#"{
            "mcpServers": {
                "slow-server": {
                    "url": "https://api.example.com/mcp/",
                    "timeout": 600
                }
            }
        }"#;

        let actual: McpConfig = serde_json::from_str(json).unwrap();

        match actual.mcp_servers.get(&"slow-server".to_string().into()) {
            Some(McpServerConfig::Http(server)) => {
                assert_eq!(server.url, "https://api.example.com/mcp/");
                assert_eq!(server.timeout, Some(600));
            }
            _ => panic!("Expected Http variant"),
        }
    }

    #[test]
    fn test_http_server_without_timeout() {
        use pretty_assertions::assert_eq;

        let json = r#"{
            "mcpServers": {
                "fast-server": {
                    "url": "https://api.example.com/mcp/"
                }
            }
        }"#;

        let actual: McpConfig = serde_json::from_str(json).unwrap();

        match actual.mcp_servers.get(&"fast-server".to_string().into()) {
            Some(McpServerConfig::Http(server)) => {
                assert_eq!(server.url, "https://api.example.com/mcp/");
                assert_eq!(server.timeout, None);
            }
            _ => panic!("Expected Http variant"),
        }
    }

    #[test]
    fn test_server_type() {
        use fake::{Fake, Faker};
        use pretty_assertions::assert_eq;

        let command: String = Faker.fake();
        let stdio_server = McpServerConfig::new_stdio(&command, vec![], None);
        let actual = stdio_server.server_type();
        let expected = "STDIO";
        assert_eq!(actual, expected);

        let url: String = format!("https://{}.example.com", Faker.fake::<String>());
        let http_server = McpServerConfig::new_http(&url);
        let actual = http_server.server_type();
        let expected = "HTTP";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_stdio_server_with_timeout() {
        use pretty_assertions::assert_eq;

        let json = r#"{
            "mcpServers": {
                "slow-stdio-server": {
                    "command": "node",
                    "args": ["server.js"],
                    "timeout": 600
                }
            }
        }"#;

        let actual: McpConfig = serde_json::from_str(json).unwrap();

        match actual
            .mcp_servers
            .get(&"slow-stdio-server".to_string().into())
        {
            Some(McpServerConfig::Stdio(server)) => {
                assert_eq!(server.command, "node");
                assert_eq!(server.args, vec!["server.js"]);
                assert_eq!(server.timeout, Some(600));
            }
            _ => panic!("Expected Stdio variant"),
        }
    }

    #[test]
    fn test_trust_store_unknown_when_empty() {
        let fixture = McpTrustStore::default();
        let actual = fixture.get_status(&PathBuf::from("/tmp/.mcp.json"), 42);
        let expected = McpTrustStatus::Unknown;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_trust_store_remembers_trust_decision() {
        let path = PathBuf::from("/tmp/.mcp.json");
        let mut fixture = McpTrustStore::default();
        fixture.trust(&path, 42);

        let actual = fixture.get_status(&path, 42);
        let expected = McpTrustStatus::Trusted;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_trust_store_remembers_reject_decision() {
        let path = PathBuf::from("/tmp/.mcp.json");
        let mut fixture = McpTrustStore::default();
        fixture.reject(&path, 42);

        let actual = fixture.get_status(&path, 42);
        let expected = McpTrustStatus::Rejected;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_trust_store_invalidates_on_hash_change() {
        let path = PathBuf::from("/tmp/.mcp.json");
        let mut fixture = McpTrustStore::default();
        fixture.trust(&path, 42);

        let actual = fixture.get_status(&path, 43);
        let expected = McpTrustStatus::Unknown;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_trust_store_trust_overwrites_prior_rejection() {
        let path = PathBuf::from("/tmp/.mcp.json");
        let mut fixture = McpTrustStore::default();
        fixture.reject(&path, 42);
        fixture.trust(&path, 42);

        let actual = fixture.get_status(&path, 42);
        let expected = McpTrustStatus::Trusted;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_trust_store_clear_removes_decision() {
        let path = PathBuf::from("/tmp/.mcp.json");
        let mut fixture = McpTrustStore::default();
        fixture.trust(&path, 42);
        fixture.clear(&path);

        let actual = fixture.get_status(&path, 42);
        let expected = McpTrustStatus::Unknown;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_trust_store_roundtrips_through_json() {
        let path = PathBuf::from("/tmp/.mcp.json");
        let mut fixture = McpTrustStore::default();
        fixture.trust(&path, 42);

        let json = serde_json::to_string(&fixture).unwrap();
        let restored: McpTrustStore = serde_json::from_str(&json).unwrap();

        let actual = restored.get_status(&path, 42);
        let expected = McpTrustStatus::Trusted;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_stdio_server_without_timeout() {
        use pretty_assertions::assert_eq;

        let json = r#"{
            "mcpServers": {
                "fast-stdio-server": {
                    "command": "node",
                    "args": ["server.js"]
                }
            }
        }"#;

        let actual: McpConfig = serde_json::from_str(json).unwrap();

        match actual
            .mcp_servers
            .get(&"fast-stdio-server".to_string().into())
        {
            Some(McpServerConfig::Stdio(server)) => {
                assert_eq!(server.command, "node");
                assert_eq!(server.timeout, None);
            }
            _ => panic!("Expected Stdio variant"),
        }
    }
}
