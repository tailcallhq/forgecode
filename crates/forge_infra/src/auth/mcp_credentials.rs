//! MCP OAuth Credential Storage
//!
//! Stores OAuth tokens separately from LLM provider credentials.
//! Credentials are bound to specific MCP server URLs.

use std::collections::HashMap;

use anyhow::Result;
use forge_domain::Environment;
use serde::{Deserialize, Serialize};
use tokio::fs;

/// MCP OAuth tokens for a single server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpOAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
    pub scope: Option<String>,
}

/// Client registration info (for dynamic registration)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpClientRegistration {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub client_id_issued_at: Option<u64>,
    pub client_secret_expires_at: Option<u64>,
}

/// Complete credential entry for an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCredentialEntry {
    /// Server URL (credential binding)
    pub server_url: String,
    /// OAuth tokens
    pub tokens: McpOAuthTokens,
    /// Client registration (if dynamically registered)
    pub client_registration: Option<McpClientRegistration>,
}

/// Credential store for all MCP servers
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpCredentialStore {
    pub credentials: HashMap<String, McpCredentialEntry>,
}

impl McpCredentialStore {
    /// Load the credential store from disk
    ///
    /// # Arguments
    /// * `env` - The environment containing the base path for storage
    ///
    /// # Returns
    /// * `Ok(McpCredentialStore)` - The loaded store, or empty if file doesn't
    ///   exist
    /// * `Err(...)` - If there was an error reading or parsing the file
    pub async fn load(env: &Environment) -> Result<Self> {
        let path = Self::credential_path(env);
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path).await?;
        Ok(serde_json::from_str(&content)?)
    }

    /// Save the credential store to disk
    ///
    /// Sets file permissions to 0o600 (user read/write only) on Unix systems.
    ///
    /// # Arguments
    /// * `env` - The environment containing the base path for storage
    pub async fn save(&self, env: &Environment) -> Result<()> {
        let path = Self::credential_path(env);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content).await?;

        // Set permissions to 0o600 (user read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&path).await?;
            let mut perms = metadata.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&path, perms).await?;
        }
        Ok(())
    }

    /// Get the path to the credential file
    ///
    /// The file is stored at `<base_path>/.mcp-credentials.json`
    pub fn credential_path(env: &Environment) -> std::path::PathBuf {
        env.base_path.join(".mcp-credentials.json")
    }

    /// Get a credential entry by server URL
    pub fn get(&self, server_url: &str) -> Option<&McpCredentialEntry> {
        self.credentials.get(server_url)
    }

    /// Set a credential entry
    pub fn set(&mut self, entry: McpCredentialEntry) {
        self.credentials.insert(entry.server_url.clone(), entry);
    }

    /// Remove a credential entry by server URL
    pub fn remove(&mut self, server_url: &str) {
        self.credentials.remove(server_url);
    }

    /// Check if credentials exist for a server URL
    #[allow(dead_code)]
    pub fn has_credentials(&self, server_url: &str) -> bool {
        self.credentials.contains_key(server_url)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    fn test_env(base_path: PathBuf) -> Environment {
        Environment {
            os: "test".to_string(),
            cwd: PathBuf::from("/tmp"),
            home: Some(PathBuf::from("/home/test")),
            shell: "bash".to_string(),
            base_path,
        }
    }

    #[tokio::test]
    async fn test_credential_store_save_load() {
        let tmp = TempDir::new().unwrap();
        let env = test_env(tmp.path().to_path_buf());
        let mut store = McpCredentialStore::default();

        let entry = McpCredentialEntry {
            server_url: "https://api.example.com/mcp".to_string(),
            tokens: McpOAuthTokens {
                access_token: "test-access-token".to_string(),
                refresh_token: Some("test-refresh-token".to_string()),
                expires_at: Some(1234567890),
                scope: Some("read write".to_string()),
            },
            client_registration: None,
        };

        store.set(entry);
        store.save(&env).await.unwrap();

        let loaded = McpCredentialStore::load(&env).await.unwrap();
        assert!(loaded.has_credentials("https://api.example.com/mcp"));
        let entry = loaded.get("https://api.example.com/mcp").unwrap();
        assert_eq!(entry.tokens.access_token, "test-access-token");
        assert_eq!(
            entry.tokens.refresh_token,
            Some("test-refresh-token".to_string())
        );
    }

    #[tokio::test]
    async fn test_credential_store_remove() {
        let tmp = TempDir::new().unwrap();
        let env = test_env(tmp.path().to_path_buf());
        let mut store = McpCredentialStore::default();

        store.set(McpCredentialEntry {
            server_url: "https://api.example.com/mcp".to_string(),
            tokens: McpOAuthTokens {
                access_token: "test".to_string(),
                refresh_token: None,
                expires_at: None,
                scope: None,
            },
            client_registration: None,
        });

        store.remove("https://api.example.com/mcp");
        assert!(!store.has_credentials("https://api.example.com/mcp"));

        // Ensure the env reference is retained to keep the TempDir alive.
        let _ = McpCredentialStore::credential_path(&env);
    }

    #[tokio::test]
    async fn test_credential_store_empty() {
        let tmp = TempDir::new().unwrap();
        let env = test_env(tmp.path().to_path_buf());
        let store = McpCredentialStore::load(&env).await.unwrap();
        assert!(store.credentials.is_empty());
    }
}
