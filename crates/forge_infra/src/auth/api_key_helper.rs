use std::time::Duration;

use chrono::{DateTime, Utc};
use forge_domain::{ApiKey, ApiKeyProvider};

/// Default timeout for helper command execution.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Returns the configured helper command timeout, read from the
/// `FORGE_API_KEY_HELPER_TIMEOUT` environment variable.  Falls back to
/// [`DEFAULT_TIMEOUT_SECS`] when the variable is absent or unparseable.
fn helper_timeout() -> Duration {
    std::env::var("FORGE_API_KEY_HELPER_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
}

/// Executes an [`ApiKeyProvider`] to obtain a fresh key.
///
/// - `StaticKey` — returns the provider unchanged (no-op).
/// - `HelperCommand` — runs the shell command via `sh -c`, parses stdout, and
///   returns an updated provider with the new key and optional expiry.  The
///   command is killed if it exceeds the configured timeout.
pub async fn execute(provider: &ApiKeyProvider) -> anyhow::Result<ApiKeyProvider> {
    match provider {
        ApiKeyProvider::StaticKey(_) => Ok(provider.clone()),
        ApiKeyProvider::HelperCommand { command, .. } => {
            let timeout = helper_timeout();

            let output = tokio::time::timeout(
                timeout,
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "Auth helper '{command}' timed out after {}s",
                    timeout.as_secs()
                )
            })?
            .map_err(|e| anyhow::anyhow!("Failed to execute auth helper '{command}': {e}"))?;

            if !output.status.success() {
                anyhow::bail!(
                    "Auth helper '{command}' exited with status {}",
                    output.status
                );
            }

            let stdout = String::from_utf8(output.stdout)
                .map_err(|e| anyhow::anyhow!("Auth helper output is not valid UTF-8: {e}"))?;

            // Normalize CRLF to LF for cross-platform compatibility
            let stdout = stdout.replace("\r\n", "\n");

            let (key, expires_at) = parse_output(&stdout)?;
            Ok(ApiKeyProvider::HelperCommand {
                command: command.clone(),
                last_key: key,
                expires_at,
            })
        }
    }
}

/// Parses helper command output into an API key and optional expiry.
///
/// Format: `<api_key>` or `<api_key>\n---\nTTL: <seconds>` or
/// `<api_key>\n---\nExpires: <unix_timestamp>`.
fn parse_output(output: &str) -> anyhow::Result<(ApiKey, Option<DateTime<Utc>>)> {
    let (key_part, metadata) = match output.split_once("\n---\n") {
        Some((key, rest)) => (key, Some(rest)),
        None => (output, None),
    };

    let key = key_part.trim().to_string();
    if key.is_empty() {
        anyhow::bail!("Auth helper produced empty output");
    }

    let expires_at = if let Some(meta) = metadata {
        let meta = meta.trim();
        if let Some(secs) = meta.strip_prefix("TTL:") {
            let ttl: u64 = secs
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid TTL value: {e}"))?;
            Some(Utc::now() + chrono::Duration::seconds(ttl as i64))
        } else if let Some(ts) = meta.strip_prefix("Expires:") {
            let timestamp: i64 = ts
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid Expires timestamp: {e}"))?;
            Some(
                DateTime::from_timestamp(timestamp, 0)
                    .ok_or_else(|| anyhow::anyhow!("Invalid unix timestamp: {timestamp}"))?,
            )
        } else {
            None
        }
    } else {
        None
    };

    Ok((ApiKey::from(key), expires_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    mod parse_output {
        use super::*;

        #[test]
        fn key_only() {
            let (key, expires_at) = parse_output("sk-test-key\n").unwrap();
            assert_eq!(key.as_ref(), "sk-test-key");
            assert!(expires_at.is_none());
        }

        #[test]
        fn key_with_ttl() {
            let (key, expires_at) = parse_output("sk-test-key\n---\nTTL: 3600\n").unwrap();
            assert_eq!(key.as_ref(), "sk-test-key");
            let exp = expires_at.unwrap();
            let expected = Utc::now() + chrono::Duration::seconds(3600);
            assert!((exp - expected).num_seconds().abs() < 5);
        }

        #[test]
        fn key_with_expires() {
            let future_ts = Utc::now().timestamp() + 7200;
            let input = format!("sk-test-key\n---\nExpires: {future_ts}\n");
            let (key, expires_at) = parse_output(&input).unwrap();
            assert_eq!(key.as_ref(), "sk-test-key");
            let exp = expires_at.unwrap();
            let expected = DateTime::from_timestamp(future_ts, 0).unwrap();
            assert!((exp - expected).num_seconds().abs() < 5);
        }

        #[test]
        fn empty_output_returns_error() {
            assert!(parse_output("  \n").is_err());
        }

        #[test]
        fn unknown_metadata_ignored() {
            let (key, expires_at) = parse_output("sk-key\n---\nFoo: bar\n").unwrap();
            assert_eq!(key.as_ref(), "sk-key");
            assert!(expires_at.is_none());
        }

        #[test]
        fn crlf_line_endings() {
            let (key, expires_at) =
                parse_output("sk-test-key\r\n---\r\nTTL: 3600\r\n").unwrap();
            assert_eq!(key.as_ref(), "sk-test-key");
            assert!(expires_at.is_some());
        }
    }

    mod execute {
        use super::*;

        #[tokio::test]
        async fn static_key_returns_unchanged() {
            let provider = ApiKeyProvider::StaticKey(ApiKey::from("sk-static".to_string()));
            let result = execute(&provider).await.unwrap();
            assert_eq!(result, provider);
        }

        #[tokio::test]
        async fn helper_command_returns_key() {
            let provider = ApiKeyProvider::HelperCommand {
                command: "echo sk-from-helper".to_string(),
                last_key: ApiKey::from("old".to_string()),
                expires_at: None,
            };
            let result = execute(&provider).await.unwrap();
            match &result {
                ApiKeyProvider::HelperCommand { last_key, .. } => {
                    assert_eq!(last_key.as_ref(), "sk-from-helper");
                }
                _ => panic!("Expected HelperCommand"),
            }
        }

        #[tokio::test]
        async fn failing_command_returns_error() {
            let provider = ApiKeyProvider::HelperCommand {
                command: "false".to_string(),
                last_key: ApiKey::from("old".to_string()),
                expires_at: None,
            };
            assert!(execute(&provider).await.is_err());
        }
    }
}
