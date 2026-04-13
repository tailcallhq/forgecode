//! Token Storage Adapter for rmcp OAuth
//!
//! Implements rmcp's `CredentialStore` trait to persist credentials
//! using our McpCredentialStore.

use std::sync::Arc;

use async_trait::async_trait;
use forge_domain::Environment;
use rmcp::transport::auth::{CredentialStore, StoredCredentials};
use tokio::sync::Mutex;

use crate::auth::mcp_credentials::{
    McpClientRegistration, McpCredentialEntry, McpCredentialStore, McpOAuthTokens,
};

/// Adapter that implements rmcp's CredentialStore trait
/// using our file-based McpCredentialStore
pub struct McpTokenStorage {
    server_url: String,
    env: Environment,
    store: Arc<Mutex<Option<McpCredentialStore>>>,
}

impl McpTokenStorage {
    /// Create a new token storage adapter for a specific MCP server
    ///
    /// # Arguments
    /// * `server_url` - The URL of the MCP server (used as credential key)
    /// * `env` - The environment for file system paths
    pub fn new(server_url: String, env: Environment) -> Self {
        Self { server_url, env, store: Arc::new(Mutex::new(None)) }
    }

    /// Get the credential store, loading it if necessary
    async fn get_store(&self) -> anyhow::Result<McpCredentialStore> {
        let mut guard = self.store.lock().await;
        if guard.is_none() {
            *guard = Some(McpCredentialStore::load(&self.env).await?);
        }
        Ok(guard.as_ref().unwrap().clone())
    }

    /// Update the cached store after modifications
    async fn update_store(&self, store: McpCredentialStore) {
        *self.store.lock().await = Some(store);
    }

    /// Load stored credentials for this server
    ///
    /// Returns the stored credential entry if one exists, or None.
    pub async fn load_credentials(&self) -> anyhow::Result<Option<McpCredentialEntry>> {
        let store = self.get_store().await?;
        Ok(store.get(&self.server_url).cloned())
    }

    /// Remove stored credentials for this server.
    pub async fn remove_credentials(&self) -> anyhow::Result<()> {
        let mut store = self.get_store().await?;
        store.remove(&self.server_url);
        store.save(&self.env).await?;
        self.update_store(store).await;
        Ok(())
    }

    /// Remove only tokens while keeping client registration.
    /// Useful when tokens are expired/invalid but the client registration
    /// (from dynamic registration) is still valid.
    #[allow(dead_code)]
    pub async fn remove_tokens_only(&self) -> anyhow::Result<()> {
        let mut store = self.get_store().await?;
        if let Some(entry) = store.get(&self.server_url).cloned() {
            let updated = McpCredentialEntry {
                server_url: entry.server_url,
                tokens: McpOAuthTokens::default(),
                client_registration: entry.client_registration,
            };
            store.set(updated);
            store.save(&self.env).await?;
            self.update_store(store).await;
        }
        Ok(())
    }
}

#[async_trait]
impl CredentialStore for McpTokenStorage {
    /// Load credentials from storage
    ///
    /// Converts our file-based credentials to rmcp's StoredCredentials format.
    /// Preserves refresh_token and expiry information for token refresh.
    async fn load(&self) -> Result<Option<StoredCredentials>, rmcp::transport::auth::AuthError> {
        let store = self
            .get_store()
            .await
            .map_err(|e| rmcp::transport::auth::AuthError::InternalError(e.to_string()))?;

        if let Some(entry) = store.get(&self.server_url) {
            use oauth2::basic::BasicTokenType;
            use oauth2::{AccessToken, RefreshToken};
            use rmcp::transport::auth::OAuthTokenResponse;

            let access_token = AccessToken::new(entry.tokens.access_token.clone());
            let token_type = BasicTokenType::Bearer;
            let extra_fields = oauth2::EmptyExtraTokenFields {};

            let mut token_response =
                OAuthTokenResponse::new(access_token, token_type, extra_fields);

            // Set refresh token if available
            if let Some(ref rt) = entry.tokens.refresh_token {
                token_response.set_refresh_token(Some(RefreshToken::new(rt.clone())));
            }

            // Set expiry if available
            if let Some(expires_at) = entry.tokens.expires_at {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if expires_at > now {
                    token_response
                        .set_expires_in(Some(&std::time::Duration::from_secs(expires_at - now)));
                } else {
                    // Token has expired - set zero duration so rmcp triggers refresh
                    token_response.set_expires_in(Some(&std::time::Duration::from_secs(0)));
                }
            }

            Ok(Some(StoredCredentials {
                client_id: entry
                    .client_registration
                    .as_ref()
                    .map(|r| r.client_id.clone())
                    .unwrap_or_default(),
                token_response: Some(token_response),
            }))
        } else {
            Ok(None)
        }
    }

    /// Save credentials to storage
    ///
    /// Converts rmcp's StoredCredentials to our file-based format,
    /// preserving all token metadata including refresh_token, expiry,
    /// and client registration info.
    async fn save(
        &self,
        credentials: StoredCredentials,
    ) -> Result<(), rmcp::transport::auth::AuthError> {
        use oauth2::TokenResponse;

        let mut store = self
            .get_store()
            .await
            .map_err(|e| rmcp::transport::auth::AuthError::InternalError(e.to_string()))?;

        // Get existing entry to preserve client_secret and registration info
        let existing = store.get(&self.server_url).cloned();

        let tokens = if let Some(ref response) = credentials.token_response {
            let access_token = response.access_token().secret().to_string();
            let refresh_token = response.refresh_token().map(|rt| rt.secret().to_string());
            let expires_at = response.expires_in().map(|d| {
                (std::time::SystemTime::now() + d)
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            });
            let scope = response.scopes().map(|scopes| {
                scopes
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            });

            McpOAuthTokens {
                access_token,
                refresh_token: refresh_token.or_else(|| {
                    existing
                        .as_ref()
                        .and_then(|e| e.tokens.refresh_token.clone())
                }),
                expires_at,
                scope,
            }
        } else {
            McpOAuthTokens {
                access_token: String::new(),
                refresh_token: None,
                expires_at: None,
                scope: None,
            }
        };

        // Preserve client_secret from dynamic registration
        let client_registration = if credentials.client_id.is_empty() {
            existing.and_then(|e| e.client_registration)
        } else {
            let existing_reg = existing
                .as_ref()
                .and_then(|e| e.client_registration.as_ref());
            Some(McpClientRegistration {
                client_id: credentials.client_id,
                client_secret: existing_reg.and_then(|r| r.client_secret.clone()),
                client_id_issued_at: existing_reg.and_then(|r| r.client_id_issued_at),
                client_secret_expires_at: existing_reg.and_then(|r| r.client_secret_expires_at),
            })
        };

        let entry = McpCredentialEntry {
            server_url: self.server_url.clone(),
            tokens,
            client_registration,
        };

        store.set(entry);
        store
            .save(&self.env)
            .await
            .map_err(|e| rmcp::transport::auth::AuthError::InternalError(e.to_string()))?;

        self.update_store(store).await;
        Ok(())
    }

    /// Clear credentials from storage
    async fn clear(&self) -> Result<(), rmcp::transport::auth::AuthError> {
        let mut store = self
            .get_store()
            .await
            .map_err(|e| rmcp::transport::auth::AuthError::InternalError(e.to_string()))?;

        store.remove(&self.server_url);
        store
            .save(&self.env)
            .await
            .map_err(|e| rmcp::transport::auth::AuthError::InternalError(e.to_string()))?;

        self.update_store(store).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn test_env() -> Environment {
        Environment {
            os: "test".to_string(),
            cwd: PathBuf::from("/tmp"),
            home: Some(PathBuf::from("/home/test")),
            shell: "bash".to_string(),
            base_path: PathBuf::from("/tmp/test-forge"),
        }
    }

    #[tokio::test]
    async fn test_token_storage_new() {
        let env = test_env();
        let storage = McpTokenStorage::new("https://example.com/mcp".to_string(), env);

        // Should start with no credentials
        let result = storage.load().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
