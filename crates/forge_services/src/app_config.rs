use std::sync::Arc;

use forge_app::{AppConfigService, EnvironmentInfra};
use forge_domain::{ConfigOperation, Effort, ModelConfig, ModelId, ProviderId, ProviderRepository};
use tracing::debug;

/// Service for managing user preferences for default providers and models.
///
/// All reads go through `infra.get_config()` so they always reflect the latest
/// on-disk state after any `update_environment` call.
pub struct ForgeAppConfigService<F> {
    infra: Arc<F>,
}

impl<F> ForgeAppConfigService<F> {
    /// Creates a new provider preferences service.
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<F: ProviderRepository + EnvironmentInfra<Config = forge_config::ForgeConfig> + Send + Sync>
    AppConfigService for ForgeAppConfigService<F>
{
    async fn get_session_config(&self) -> Option<ModelConfig> {
        let config = self.infra.get_config().ok()?;
        let is_shell = self
            .infra
            .get_env_var("FORGE_SHELL_PROMPT")
            .is_some_and(|v| v == "1");
        let source = if is_shell {
            config.shell.as_ref().or(config.session.as_ref())
        } else {
            config.session.as_ref()
        };
        let session = source?;
        Some(ModelConfig {
            provider: ProviderId::from(session.provider_id.clone()),
            model: ModelId::new(session.model_id.clone()),
        })
    }

    async fn get_commit_config(&self) -> anyhow::Result<Option<forge_domain::ModelConfig>> {
        let config = self.infra.get_config()?;
        Ok(config.commit.clone().map(|mc| ModelConfig {
            provider: ProviderId::from(mc.provider_id),
            model: ModelId::new(mc.model_id),
        }))
    }

    async fn get_shell_config(&self) -> anyhow::Result<Option<forge_domain::ModelConfig>> {
        let config = self.infra.get_config()?;
        Ok(config.shell.or(config.session).map(|mc| ModelConfig {
            provider: ProviderId::from(mc.provider_id),
            model: ModelId::new(mc.model_id),
        }))
    }

    async fn get_suggest_config(&self) -> anyhow::Result<Option<forge_domain::ModelConfig>> {
        let config = self.infra.get_config()?;
        Ok(config.suggest.clone().map(|mc| ModelConfig {
            provider: ProviderId::from(mc.provider_id),
            model: ModelId::new(mc.model_id),
        }))
    }

    async fn get_reasoning_effort(&self) -> anyhow::Result<Option<Effort>> {
        let config = self.infra.get_config()?;
        Ok(config
            .reasoning
            .clone()
            .and_then(|r| r.effort)
            .map(|e| match e {
                forge_config::Effort::None => Effort::None,
                forge_config::Effort::Minimal => Effort::Minimal,
                forge_config::Effort::Low => Effort::Low,
                forge_config::Effort::Medium => Effort::Medium,
                forge_config::Effort::High => Effort::High,
                forge_config::Effort::XHigh => Effort::XHigh,
                forge_config::Effort::Max => Effort::Max,
            }))
    }

    async fn update_config(&self, ops: Vec<ConfigOperation>) -> anyhow::Result<()> {
        debug!(ops = ?ops, "Updating app config");
        self.infra.update_environment(ops).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use forge_config::{ForgeConfig, ModelConfig};
    // Alias to avoid collision with forge_config::ModelConfig used in test fixtures
    use forge_domain::ModelConfig as DomainModelConfig;
    use forge_domain::{
        AnyProvider, ChatRepository, ConfigOperation, Environment, InputModality, MigrationResult,
        Model, ModelId, ModelSource, Provider, ProviderId, ProviderResponse, ProviderTemplate,
    };
    use pretty_assertions::assert_eq;
    use url::Url;

    use super::*;

    #[derive(Clone)]
    struct MockInfra {
        config: Arc<Mutex<ForgeConfig>>,
        providers: Vec<Provider<Url>>,
        env_vars: Arc<Mutex<HashMap<String, String>>>,
    }

    impl MockInfra {
        fn new() -> Self {
            Self {
                config: Arc::new(Mutex::new(ForgeConfig::default())),
                providers: vec![
                    Provider {
                        id: ProviderId::OPENAI,
                        provider_type: Default::default(),
                        response: Some(ProviderResponse::OpenAI),
                        url: Url::parse("https://api.openai.com").unwrap(),
                        credential: Some(forge_domain::AuthCredential {
                            id: ProviderId::OPENAI,
                            auth_details: forge_domain::AuthDetails::ApiKey(
                                forge_domain::ApiKey::from("test-key".to_string()),
                            ),
                            url_params: HashMap::new(),
                        }),
                        auth_methods: vec![forge_domain::AuthMethod::ApiKey],
                        url_params: vec![],
                        models: Some(ModelSource::Hardcoded(vec![Model {
                            id: "gpt-4".to_string().into(),
                            name: Some("GPT-4".to_string()),
                            description: None,
                            context_length: Some(8192),
                            tools_supported: Some(true),
                            supports_parallel_tool_calls: Some(true),
                            supports_reasoning: Some(false),
                            supported_reasoning_efforts: None,
                            input_modalities: vec![InputModality::Text],
                        }])),
                        custom_headers: None,
                    },
                    Provider {
                        id: ProviderId::ANTHROPIC,
                        provider_type: Default::default(),
                        response: Some(ProviderResponse::Anthropic),
                        url: Url::parse("https://api.anthropic.com").unwrap(),
                        auth_methods: vec![forge_domain::AuthMethod::ApiKey],
                        url_params: vec![],
                        credential: Some(forge_domain::AuthCredential {
                            id: ProviderId::ANTHROPIC,
                            auth_details: forge_domain::AuthDetails::ApiKey(
                                forge_domain::ApiKey::from("test-key".to_string()),
                            ),
                            url_params: HashMap::new(),
                        }),
                        models: Some(ModelSource::Hardcoded(vec![Model {
                            id: "claude-3".to_string().into(),
                            name: Some("Claude 3".to_string()),
                            description: None,
                            context_length: Some(200000),
                            tools_supported: Some(true),
                            supports_parallel_tool_calls: Some(true),
                            supports_reasoning: Some(true),
                            supported_reasoning_efforts: None,
                            input_modalities: vec![InputModality::Text],
                        }])),
                        custom_headers: None,
                    },
                ],
                env_vars: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        fn with_env_var(self, key: &str, value: &str) -> Self {
            self.env_vars.lock().unwrap().insert(key.to_string(), value.to_string());
            self
        }
    }

    impl EnvironmentInfra for MockInfra {
        type Config = ForgeConfig;

        fn get_environment(&self) -> Environment {
            Environment {
                os: "test".to_string(),
                cwd: PathBuf::new(),
                home: None,
                shell: "bash".to_string(),
                base_path: PathBuf::new(),
            }
        }

        fn update_environment(
            &self,
            ops: Vec<ConfigOperation>,
        ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send {
            let config = self.config.clone();
            async move {
                let mut config = config.lock().unwrap();
                for op in ops {
                    match op {
                        ConfigOperation::SetSessionConfig(mc) => {
                            let pid_str = mc.provider.as_ref().to_string();
                            let mid_str = mc.model.to_string();
                            config.session = Some(ModelConfig::new(pid_str, mid_str));
                        }
                        ConfigOperation::SetShellConfig(mc) => {
                            config.shell = Some(ModelConfig::new(
                                mc.provider.as_ref().to_string(),
                                mc.model.to_string(),
                            ));
                        }
                        ConfigOperation::SetCommitConfig(mc) => {
                            config.commit = mc.map(|m| {
                                ModelConfig::new(
                                    m.provider.as_ref().to_string(),
                                    m.model.to_string(),
                                )
                            });
                        }
                        ConfigOperation::SetSuggestConfig(mc) => {
                            config.suggest = Some(ModelConfig::new(
                                mc.provider.as_ref().to_string(),
                                mc.model.to_string(),
                            ));
                        }
                        ConfigOperation::SetReasoningEffort(_) => {
                            // No-op in tests
                        }
                        ConfigOperation::SetSudo(_) => {
                            // Sudo is session-scoped, not persisted to config
                        }
                    }
                }
                Ok(())
            }
        }

        fn get_config(&self) -> anyhow::Result<ForgeConfig> {
            Ok(self.config.lock().unwrap().clone())
        }

        fn get_env_var(&self, key: &str) -> Option<String> {
            self.env_vars.lock().unwrap().get(key).cloned()
        }

        fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
            self.env_vars.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        }
    }

    #[async_trait::async_trait]
    impl ChatRepository for MockInfra {
        async fn chat(
            &self,
            _model_id: &forge_app::domain::ModelId,
            _context: forge_app::domain::Context,
            _provider: Provider<Url>,
        ) -> forge_app::domain::ResultStream<forge_app::domain::ChatCompletionMessage, anyhow::Error>
        {
            Ok(Box::pin(tokio_stream::iter(vec![])))
        }

        async fn models(
            &self,
            _provider: Provider<Url>,
        ) -> anyhow::Result<Vec<forge_app::domain::Model>> {
            Ok(vec![])
        }
    }

    #[async_trait::async_trait]
    impl ProviderRepository for MockInfra {
        async fn get_all_providers(&self) -> anyhow::Result<Vec<AnyProvider>> {
            Ok(self
                .providers
                .iter()
                .map(|p| AnyProvider::Url(p.clone()))
                .collect())
        }

        async fn get_provider(&self, id: ProviderId) -> anyhow::Result<ProviderTemplate> {
            // Convert Provider<Url> to Provider<Template<...>> for testing
            self.providers
                .iter()
                .find(|p| p.id == id)
                .map(|p| Provider {
                    id: p.id.clone(),
                    provider_type: p.provider_type,
                    response: p.response.clone(),
                    url: forge_domain::Template::<forge_domain::URLParameters>::new(p.url.as_str()),
                    models: p.models.as_ref().map(|m| match m {
                        ModelSource::Url(url) => ModelSource::Url(forge_domain::Template::<
                            forge_domain::URLParameters,
                        >::new(
                            url.as_str()
                        )),
                        ModelSource::Hardcoded(list) => ModelSource::Hardcoded(list.clone()),
                    }),
                    auth_methods: p.auth_methods.clone(),
                    url_params: p.url_params.clone(),
                    credential: p.credential.clone(),
                    custom_headers: None,
                })
                .ok_or_else(|| anyhow::anyhow!("Provider not found"))
        }

        async fn upsert_credential(
            &self,
            _credential: forge_domain::AuthCredential,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn get_credential(
            &self,
            _id: &ProviderId,
        ) -> anyhow::Result<Option<forge_domain::AuthCredential>> {
            Ok(None)
        }

        async fn remove_credential(&self, _id: &ProviderId) -> anyhow::Result<()> {
            Ok(())
        }

        async fn migrate_env_credentials(&self) -> anyhow::Result<Option<MigrationResult>> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn test_get_session_config_when_none_set() -> anyhow::Result<()> {
        let fixture = MockInfra::new();
        let service = ForgeAppConfigService::new(Arc::new(fixture));

        let result = service.get_session_config().await;

        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_session_config_when_set() -> anyhow::Result<()> {
        let fixture = MockInfra::new();
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        service
            .update_config(vec![ConfigOperation::SetSessionConfig(
                DomainModelConfig::new(ProviderId::ANTHROPIC, ModelId::new("claude-3")),
            )])
            .await?;
        let actual = service.get_session_config().await;
        let expected = Some(DomainModelConfig::new(
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3"),
        ));

        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_session_config_when_provider_not_available() -> anyhow::Result<()> {
        let mut fixture = MockInfra::new();
        // Remove OpenAI from available providers but keep it in config
        fixture.providers.retain(|p| p.id != ProviderId::OPENAI);
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        // Set OpenAI as the default provider in config (with a model)
        service
            .update_config(vec![ConfigOperation::SetSessionConfig(
                DomainModelConfig::new(ProviderId::OPENAI, ModelId::new("gpt-4")),
            )])
            .await?;

        // Should return the config even if provider is not available
        // Validation happens when getting the actual provider via ProviderService
        let result = service.get_session_config().await;

        assert_eq!(
            result,
            Some(DomainModelConfig::new(
                ProviderId::OPENAI,
                ModelId::new("gpt-4")
            ))
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_set_session_config() -> anyhow::Result<()> {
        let fixture = MockInfra::new();
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        service
            .update_config(vec![ConfigOperation::SetSessionConfig(
                DomainModelConfig::new(ProviderId::ANTHROPIC, ModelId::new("claude-3")),
            )])
            .await?;

        let actual = service.get_session_config().await;
        let expected = Some(DomainModelConfig::new(
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3"),
        ));

        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_shell_config_falls_back_to_session_config() -> anyhow::Result<()> {
        let fixture = MockInfra::new();
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        service
            .update_config(vec![ConfigOperation::SetSessionConfig(
                DomainModelConfig::new(ProviderId::ANTHROPIC, ModelId::new("claude-3")),
            )])
            .await?;
        let actual = service.get_shell_config().await?;
        let expected = Some(DomainModelConfig::new(
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3"),
        ));

        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_shell_config_prefers_explicit_shell_config() -> anyhow::Result<()> {
        let fixture = MockInfra::new();
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        service
            .update_config(vec![
                ConfigOperation::SetSessionConfig(DomainModelConfig::new(
                    ProviderId::ANTHROPIC,
                    ModelId::new("claude-3"),
                )),
                ConfigOperation::SetShellConfig(DomainModelConfig::new(
                    ProviderId::OPENAI,
                    ModelId::new("gpt-4"),
                )),
            ])
            .await?;
        let actual = service.get_shell_config().await?;
        let expected = Some(DomainModelConfig::new(
            ProviderId::OPENAI,
            ModelId::new("gpt-4"),
        ));

        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_session_config_model_when_none_set() -> anyhow::Result<()> {
        let fixture = MockInfra::new();
        let service = ForgeAppConfigService::new(Arc::new(fixture));

        let result = service.get_session_config().await;

        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_session_config_model_when_set() -> anyhow::Result<()> {
        let fixture = MockInfra::new();
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        service
            .update_config(vec![ConfigOperation::SetSessionConfig(
                DomainModelConfig::new(ProviderId::OPENAI, ModelId::new("gpt-4")),
            )])
            .await?;
        let actual = service.get_session_config().await.map(|c| c.model);
        let expected = Some(ModelId::new("gpt-4"));

        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_set_session_config_model() -> anyhow::Result<()> {
        let fixture = MockInfra::new();
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        service
            .update_config(vec![ConfigOperation::SetSessionConfig(
                DomainModelConfig::new(ProviderId::OPENAI, ModelId::from("gpt-4".to_string())),
            )])
            .await?;

        let actual = service.get_session_config().await.map(|c| c.model);
        let expected = Some(ModelId::from("gpt-4".to_string()));

        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_set_multiple_default_models() -> anyhow::Result<()> {
        let fixture = MockInfra::new();
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        // Set model for OpenAI first
        service
            .update_config(vec![ConfigOperation::SetSessionConfig(
                DomainModelConfig::new(ProviderId::OPENAI, ModelId::from("gpt-4".to_string())),
            )])
            .await?;

        // Then switch to Anthropic with its model
        service
            .update_config(vec![ConfigOperation::SetSessionConfig(
                DomainModelConfig::new(
                    ProviderId::ANTHROPIC,
                    ModelId::from("claude-3".to_string()),
                ),
            )])
            .await?;

        // ForgeConfig only tracks a single active session, so the last
        // provider/model pair wins
        let actual = service.get_session_config().await;
        let expected = Some(DomainModelConfig::new(
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3"),
        ));

        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_session_config_prefers_shell_config_in_shell_mode() -> anyhow::Result<()> {
        let fixture = MockInfra::new().with_env_var("FORGE_SHELL_PROMPT", "1");
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        // Set both session and shell config with different models
        service
            .update_config(vec![
                ConfigOperation::SetSessionConfig(DomainModelConfig::new(
                    ProviderId::ANTHROPIC,
                    ModelId::new("claude-3"),
                )),
                ConfigOperation::SetShellConfig(DomainModelConfig::new(
                    ProviderId::OPENAI,
                    ModelId::new("gpt-4"),
                )),
            ])
            .await?;

        // In shell mode, get_session_config should prefer shell config
        let actual = service.get_session_config().await;
        let expected = Some(DomainModelConfig::new(
            ProviderId::OPENAI,
            ModelId::new("gpt-4"),
        ));

        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_session_config_falls_back_to_session_in_shell_mode() -> anyhow::Result<()> {
        let fixture = MockInfra::new().with_env_var("FORGE_SHELL_PROMPT", "1");
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        // Set only session config (no shell config)
        service
            .update_config(vec![ConfigOperation::SetSessionConfig(
                DomainModelConfig::new(ProviderId::ANTHROPIC, ModelId::new("claude-3")),
            )])
            .await?;

        // In shell mode, should fall back to session config when shell config is absent
        let actual = service.get_session_config().await;
        let expected = Some(DomainModelConfig::new(
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3"),
        ));

        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_session_config_ignores_shell_config_outside_shell_mode() -> anyhow::Result<()> {
        let fixture = MockInfra::new(); // No FORGE_SHELL_PROMPT
        let service = ForgeAppConfigService::new(Arc::new(fixture.clone()));

        // Set both session and shell config
        service
            .update_config(vec![
                ConfigOperation::SetSessionConfig(DomainModelConfig::new(
                    ProviderId::ANTHROPIC,
                    ModelId::new("claude-3"),
                )),
                ConfigOperation::SetShellConfig(DomainModelConfig::new(
                    ProviderId::OPENAI,
                    ModelId::new("gpt-4"),
                )),
            ])
            .await?;

        // Outside shell mode, get_session_config should return session config
        let actual = service.get_session_config().await;
        let expected = Some(DomainModelConfig::new(
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3"),
        ));

        assert_eq!(actual, expected);
        Ok(())
    }
}
