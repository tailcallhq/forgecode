//! GitHub App authentication + API client.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// GitHub App installation token response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallationToken {
    pub token: String,
    pub expires_at: String,
}

/// Lightweight GitHub API client backed by a cached installation token.
///
/// We keep the dependency surface tiny — only `reqwest` is required for
/// HTTP, and the token cache lives in-process.
#[allow(dead_code)]
#[derive(Clone)]
pub struct GitHubClient {
    app_id: u64,
    installation_id: u64,
    /// Cached installation token. `None` means "needs refresh".
    cached_token: std::sync::Arc<std::sync::Mutex<Option<InstallationToken>>>,
    http: reqwest::Client,
}

#[allow(dead_code)]
impl GitHubClient {
    pub fn new(app_id: u64, installation_id: u64) -> Self {
        Self {
            app_id,
            installation_id,
            cached_token: std::sync::Arc::new(std::sync::Mutex::new(None)),
            http: reqwest::Client::new(),
        }
    }

    /// Get a valid installation token, refreshing if expired or missing.
    pub async fn token(&self) -> Result<String> {
        // Check the cache first.
        {
            let guard = self.cached_token.lock().unwrap();
            if let Some(tok) = guard.as_ref()
                && !is_expired(&tok.expires_at)
            {
                return Ok(tok.token.clone());
            }
        }

        // Refresh via the GitHub App API.
        let token = self.refresh_token().await?;
        let mut guard = self.cached_token.lock().unwrap();
        *guard = Some(token.clone());
        Ok(token.token)
    }

    async fn refresh_token(&self) -> Result<InstallationToken> {
        // Real implementation: POST https://api.github.com/app/installations/{id}/access_tokens
        // with a JWT signed by the app's private key.
        //
        // Stub: in the absence of a configured JWT signer, return a synthetic
        // token.  Production deployments must override this method via
        // `GitHubClient::with_jwt_signer`.
        anyhow::bail!(
            "GitHubClient::refresh_token() requires a JWT signer; use \
             GitHubClient::with_jwt_signer() in production"
        )
    }

    /// Post a comment on an issue/PR.
    pub async fn post_comment(&self, repo: &str, issue_number: u64, body: &str) -> Result<u64> {
        let token = self.token().await?;
        let url = format!("https://api.github.com/repos/{repo}/issues/{issue_number}/comments");
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .context("POST comment")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("comment POST failed: {status} {text}");
        }
        let json: serde_json::Value = resp.json().await.context("decode comment response")?;
        let id = json.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(id)
    }

    /// Get the diff/PATCH for an issue or PR (used to give the agent context).
    pub async fn get_issue_body(&self, repo: &str, issue_number: u64) -> Result<String> {
        let token = self.token().await?;
        let url = format!("https://api.github.com/repos/{repo}/issues/{issue_number}");
        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .context("GET issue")?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("issue GET failed: {status}");
        }
        let json: serde_json::Value = resp.json().await.context("decode issue")?;
        let body = json.get("body").and_then(|v| v.as_str()).unwrap_or("");
        Ok(body.to_string())
    }

    pub fn app_id(&self) -> u64 {
        self.app_id
    }
}

#[allow(dead_code)]
fn is_expired(_expires_at: &str) -> bool {
    // Stub: never expire. Real impl parses the ISO-8601 timestamp and
    // compares against current UTC.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_constructs() {
        let client = GitHubClient::new(12345, 67890);
        assert_eq!(client.app_id(), 12345);
    }
}
