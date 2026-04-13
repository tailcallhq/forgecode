use derive_setters::Setters;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for retry mechanism.
#[derive(
    Default, Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy, Setters,
)]
#[serde(rename_all = "snake_case")]
#[setters(into)]
pub struct RetryConfig {
    /// Initial backoff delay in milliseconds for retry operations
    pub initial_backoff_ms: u64,
    /// Minimum delay in milliseconds between retry attempts
    pub min_delay_ms: u64,
    /// Backoff multiplication factor for each retry attempt
    pub backoff_factor: u64,
    /// Maximum number of retry attempts
    pub max_attempts: usize,
    /// HTTP status codes that should trigger retries
    pub status_codes: Vec<u16>,
    /// Maximum delay between retries in seconds
    pub max_delay_secs: Option<u64>,
    /// Whether to suppress retry error logging and events
    pub suppress_errors: bool,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_retry_config_fields() {
        let config = RetryConfig {
            initial_backoff_ms: 200,
            min_delay_ms: 1000,
            backoff_factor: 2,
            max_attempts: 8,
            status_codes: vec![429, 500, 502, 503, 504, 408, 522, 524, 520, 529],
            max_delay_secs: None,
            suppress_errors: false,
        };
        assert_eq!(config.initial_backoff_ms, 200);
        assert_eq!(config.suppress_errors, false);
    }
}
