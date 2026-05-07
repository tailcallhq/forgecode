use std::sync::Arc;
use std::time::Duration;

use bstr::ByteSlice;
use forge_app::{AuthStrategy, ProviderAuthService, StrategyFactory};
use forge_domain::{
    AuthContextRequest, AuthContextResponse, AuthMethod, Provider, ProviderId, ProviderRepository,
};

/// Forge Provider Authentication Service
#[derive(Clone)]
pub struct ForgeProviderAuthService<I> {
    infra: Arc<I>,
}

impl<I> ForgeProviderAuthService<I> {
    /// Create a new provider authentication service
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }

    /// Execute an api_key_command and return the trimmed output as an ApiKey.
    async fn execute_api_key_command(command: &str) -> anyhow::Result<forge_domain::ApiKey> {
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

        Ok(forge_domain::ApiKey::from(token))
    }
}

#[async_trait::async_trait]
impl<I> ProviderAuthService for ForgeProviderAuthService<I>
where
    I: StrategyFactory + ProviderRepository + Send + Sync + 'static,
{
    /// Initialize authentication flow for a provider
    async fn init_provider_auth(
        &self,
        provider_id: ProviderId,
        auth_method: AuthMethod,
    ) -> anyhow::Result<AuthContextRequest> {
        // Get required URL parameters for API key flow and Google ADC
        let required_params = if matches!(
            auth_method,
            AuthMethod::ApiKey
                | AuthMethod::GoogleAdc
                | AuthMethod::AwsProfile
                | AuthMethod::Command { .. }
        ) {
            // Get URL params from provider entry (works for both configured and
            // unconfigured)
            let providers = self.infra.get_all_providers().await?;
            let provider = providers
                .iter()
                .find(|p| p.id() == provider_id)
                .ok_or_else(|| forge_domain::Error::provider_not_available(provider_id.clone()))?;
            provider.url_params().to_vec()
        } else {
            vec![]
        };

        // Create appropriate strategy and initialize
        let strategy = self.infra.create_auth_strategy(
            provider_id.clone(),
            auth_method.clone(),
            required_params,
        )?;
        let mut request = strategy.init().await?;

        // For API key flow and Google ADC, attach existing credential if available
        if let AuthContextRequest::ApiKey(ref mut api_key_request) = request
            && let Ok(Some(existing_credential)) = self.infra.get_credential(&provider_id).await
        {
            api_key_request.existing_params = Some(existing_credential.url_params.into());

            // Only prefill API key for regular API Key flow
            // Don't overwrite markers (google_adc_marker, aws_profile_marker)
            // used by non-API-key auth methods
            if !matches!(auth_method, AuthMethod::GoogleAdc | AuthMethod::AwsProfile)
                && let Some(key) = existing_credential.auth_details.api_key()
            {
                let is_adc_marker = key.as_ref() == "google_adc_marker";
                if !is_adc_marker {
                    api_key_request.api_key = Some(key.clone());
                }
            }
        }

        Ok(request)
    }

    /// Complete authentication flow for a provider
    async fn complete_provider_auth(
        &self,
        provider_id: ProviderId,
        auth_context_response: AuthContextResponse,
        _timeout: Duration,
    ) -> anyhow::Result<()> {
        // Extract auth method from context response
        // For ApiKey responses, we need to check if it's Google ADC or regular API key
        let auth_method = match &auth_context_response {
            AuthContextResponse::ApiKey(response) => {
                // Check if provider supports Google ADC and if it's the Google ADC marker
                let is_vertex_provider = provider_id == forge_domain::ProviderId::VERTEX_AI
                    || provider_id == forge_domain::ProviderId::VERTEX_AI_ANTHROPIC;
                if is_vertex_provider && response.response.api_key.as_ref() == "google_adc_marker" {
                    // Vertex AI uses Google ADC
                    forge_domain::AuthMethod::google_adc()
                } else if response.response.api_key.as_ref() == "aws_profile_marker" {
                    // AWS Profile authentication
                    forge_domain::AuthMethod::AwsProfile
                } else {
                    // Regular API key
                    forge_domain::AuthMethod::ApiKey
                }
            }
            AuthContextResponse::Code(ctx) => {
                AuthMethod::OAuthCode(ctx.request.oauth_config.clone())
            }
            AuthContextResponse::DeviceCode(ctx) => {
                if provider_id == forge_domain::ProviderId::CODEX {
                    AuthMethod::CodexDevice(ctx.request.oauth_config.clone())
                } else {
                    AuthMethod::OAuthDevice(ctx.request.oauth_config.clone())
                }
            }
        };

        // Get required params for API key flow
        let required_params = if matches!(auth_method, AuthMethod::ApiKey) {
            // Get URL params from provider entry (works for both configured and
            // unconfigured)
            let providers = self.infra.get_all_providers().await?;
            let provider = providers
                .iter()
                .find(|p| p.id() == provider_id)
                .ok_or_else(|| forge_domain::Error::provider_not_available(provider_id.clone()))?;
            provider.url_params().to_vec()
        } else {
            vec![]
        };

        // Create strategy and complete authentication
        let strategy =
            self.infra
                .create_auth_strategy(provider_id.clone(), auth_method, required_params)?;
        let credential = strategy.complete(auth_context_response).await?;

        // Store credential
        self.infra.upsert_credential(credential).await
    }

    /// Refreshes provider credentials if they're about to expire.
    /// Checks if credential needs refresh (5 minute buffer before expiry),
    /// iterates through provider's auth methods, and attempts to refresh.
    /// Returns the provider with updated credentials, or original if refresh
    /// fails or isn't needed.
    async fn refresh_provider_credential(
        &self,
        mut provider: Provider<url::Url>,
    ) -> anyhow::Result<Provider<url::Url>> {
        // Handle command_no_cache: always re-execute regardless of needs_refresh
        for auth_method in &provider.auth_methods {
            if let AuthMethod::Command { command, cache: false } = auth_method {
                match Self::execute_api_key_command(command).await {
                    Ok(api_key) => {
                        provider.credential = Some(
                            forge_domain::AuthCredential::new_api_key(
                                provider.id.clone(),
                                api_key,
                            ),
                        );
                        return Ok(provider);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to refresh command auth for {}: {e}",
                            provider.id
                        );
                        // Continue with existing credential
                    }
                }
            }
        }

        // Check if credential needs refresh (5 minute buffer before expiry)
        if let Some(credential) = &provider.credential {
            let buffer = chrono::Duration::minutes(5);

            if credential.needs_refresh(buffer) {
                // Iterate through auth methods and try to refresh
                for auth_method in &provider.auth_methods {
                    match auth_method {
                        AuthMethod::OAuthDevice(_)
                        | AuthMethod::OAuthCode(_)
                        | AuthMethod::CodexDevice(_)
                        | AuthMethod::GoogleAdc => {
                            // Get existing credential
                            let existing_credential =
                                self.infra.get_credential(&provider.id).await?.ok_or_else(
                                    || forge_domain::Error::ProviderNotAvailable {
                                        provider: provider.id.clone(),
                                    },
                                )?;

                            // Get required params (only used for API key, but needed for factory)
                            let required_params = if matches!(auth_method, AuthMethod::ApiKey) {
                                provider.url_params.clone()
                            } else {
                                vec![]
                            };

                            // Create strategy and refresh credential
                            if let Ok(strategy) = self.infra.create_auth_strategy(
                                provider.id.clone(),
                                auth_method.clone(),
                                required_params,
                            ) {
                                match strategy.refresh(&existing_credential).await {
                                    Ok(refreshed) => {
                                        // Store refreshed credential
                                        if self
                                            .infra
                                            .upsert_credential(refreshed.clone())
                                            .await
                                            .is_err()
                                        {
                                            continue;
                                        }

                                        // Update provider with refreshed credential
                                        provider.credential = Some(refreshed);
                                        break; // Success, stop trying other methods
                                    }
                                    Err(_) => {
                                        // If refresh fails, continue with
                                        // existing credentials
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(provider)
    }
}
