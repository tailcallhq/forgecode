use bstr::ByteSlice;
use forge_app::AuthStrategy;
use forge_domain::{
    ApiKey, ApiKeyRequest, AuthContextRequest, AuthContextResponse, AuthCredential, ProviderId,
};
/// Executes a shell command and returns the trimmed stdout as an `ApiKey`.
pub async fn execute_api_key_command(command: &str) -> anyhow::Result<ApiKey> {
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to execute api_key_command `{command}`: {e}"))?;

    if !output.status.success() {
        let stderr = output.stderr.as_slice().to_str_lossy();
        anyhow::bail!(
            "api_key_command `{command}` exited with {}: {}",
            output.status,
            stderr.trim()
        );
    }

    let token = String::from_utf8(output.stdout)
        .map_err(|e| anyhow::anyhow!("api_key_command output is not valid UTF-8: {e}"))?
        .trim()
        .to_string();

    if token.is_empty() {
        anyhow::bail!("api_key_command `{command}` produced no output");
    }

    Ok(ApiKey::from(token))
}

/// Auth strategy for command-based authentication.
/// Auto-resolves by executing the configured command — no user interaction
/// needed.
pub struct CommandAuthStrategy {
    provider_id: ProviderId,
    command: String,
}

impl CommandAuthStrategy {
    /// Creates a new command-based auth strategy for the given provider.
    pub fn new(provider_id: ProviderId, command: String) -> Self {
        Self { provider_id, command }
    }
}

#[async_trait::async_trait]
impl AuthStrategy for CommandAuthStrategy {
    async fn init(&self) -> anyhow::Result<AuthContextRequest> {
        let api_key = execute_api_key_command(&self.command).await?;
        Ok(AuthContextRequest::ApiKey(ApiKeyRequest {
            api_key: Some(api_key),
            required_params: vec![],
            existing_params: None,
        }))
    }

    async fn complete(
        &self,
        _context_response: AuthContextResponse,
    ) -> anyhow::Result<AuthCredential> {
        let api_key = execute_api_key_command(&self.command).await?;
        Ok(AuthCredential::new_api_key(self.provider_id.clone(), api_key))
    }

    async fn refresh(&self, _credential: &AuthCredential) -> anyhow::Result<AuthCredential> {
        let api_key = execute_api_key_command(&self.command).await?;
        Ok(AuthCredential::new_api_key(self.provider_id.clone(), api_key))
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[tokio::test]
    async fn test_execute_command_returns_trimmed_stdout() {
        let actual = execute_api_key_command("echo hello-token").await.unwrap();
        let expected = ApiKey::from("hello-token".to_string());
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_execute_command_trims_whitespace() {
        let actual = execute_api_key_command("echo '  spaced-token  '")
            .await
            .unwrap();
        let expected = ApiKey::from("spaced-token".to_string());
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_execute_command_fails_on_bad_exit_code() {
        let actual = execute_api_key_command("sh -c 'exit 1'").await;
        assert!(actual.is_err());
    }

    #[tokio::test]
    async fn test_execute_command_fails_on_empty_output() {
        let actual = execute_api_key_command("echo ''").await;
        assert!(actual.is_err());
    }

}
