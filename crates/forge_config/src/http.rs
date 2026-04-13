use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// TLS version enum for configuring TLS protocol versions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy)]
#[serde(rename_all = "snake_case")]
pub enum TlsVersion {
    #[serde(rename = "1.0")]
    V1_0,
    #[serde(rename = "1.1")]
    V1_1,
    #[serde(rename = "1.2")]
    V1_2,
    #[serde(rename = "1.3")]
    V1_3,
}

/// TLS backend option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy)]
#[serde(rename_all = "snake_case")]
pub enum TlsBackend {
    #[serde(rename = "default")]
    Default,
    #[serde(rename = "rustls")]
    Rustls,
}

/// HTTP client configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy)]
#[serde(rename_all = "snake_case")]
pub struct HttpConfig {
    pub connect_timeout_secs: u64,
    pub read_timeout_secs: u64,
    pub pool_idle_timeout_secs: u64,
    pub pool_max_idle_per_host: usize,
    pub max_redirects: usize,
    pub hickory: bool,
    pub tls_backend: TlsBackend,
    /// Minimum TLS protocol version to use
    pub min_tls_version: Option<TlsVersion>,
    /// Maximum TLS protocol version to use
    pub max_tls_version: Option<TlsVersion>,
    /// Adaptive window sizing for improved flow control
    pub adaptive_window: bool,
    /// Keep-alive interval in seconds
    pub keep_alive_interval_secs: Option<u64>,
    /// Keep-alive timeout in seconds
    pub keep_alive_timeout_secs: u64,
    /// Keep-alive while connection is idle
    pub keep_alive_while_idle: bool,
    /// Accept invalid certificates
    pub accept_invalid_certs: bool,
    /// Paths to root certificate files
    pub root_cert_paths: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_http_config_fields() {
        let config = HttpConfig {
            connect_timeout_secs: 30,
            read_timeout_secs: 900,
            pool_idle_timeout_secs: 90,
            pool_max_idle_per_host: 5,
            max_redirects: 10,
            hickory: false,
            tls_backend: TlsBackend::Default,
            min_tls_version: None,
            max_tls_version: None,
            adaptive_window: true,
            keep_alive_interval_secs: Some(60),
            keep_alive_timeout_secs: 10,
            keep_alive_while_idle: true,
            accept_invalid_certs: false,
            root_cert_paths: None,
        };
        assert_eq!(config.connect_timeout_secs, 30);
        assert_eq!(config.adaptive_window, true);
    }

    #[test]
    fn test_tls_version_variants() {
        assert_eq!(TlsVersion::V1_3, TlsVersion::V1_3);
    }

    #[test]
    fn test_tls_backend_variants() {
        assert_eq!(TlsBackend::Default, TlsBackend::Default);
    }
}
