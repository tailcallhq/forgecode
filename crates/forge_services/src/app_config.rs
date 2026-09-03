use std::sync::Arc;

use forge_app::{AppConfigService, EnvironmentInfra};
use forge_domain::{
    ConfigOperation, Effort, HeliosdoctorInfo, ModelConfig, ModelId, ProviderId, ProviderRepository,
};
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

    /// Resolves the non-stat portion of the diagnostics report: base path,
    /// db path, updater channel, binary identity, and config source. Shared
    /// by `heliosdoctor`, `heliosdoctor_verbose`, and
    /// `heliosdoctor_integrity`.
    fn heliosdoctor_base_info(&self) -> HeliosdoctorInfo {
        // Deliberately resolves the path from forge_config rather than the
        // infra's environment: this is the canonical resolution used by the
        // Gate 5 data-dir split and is identical across all binaries.
        let base_path = forge_config::ConfigReader::base_path();
        // Mirrors forge_domain::Environment::legacy_database_path: the read
        // side unions in the legacy `.forge.db` by default, and
        // FORGE_LEGACY_DB_PATH overrides which legacy file is reported.
        let legacy_db_path = if let Ok(path) = std::env::var("FORGE_LEGACY_DB_PATH") {
            std::path::PathBuf::from(path)
        } else {
            base_path.join(".forge.db")
        };
        // Mirrors forge_domain::Environment::write_database_path: the fork
        // writes to a separate ".forge.writes.db" by default while the read
        // side unions in legacy ".forge.db". FORGE_WRITE_DB_PATH overrides
        // the write target for callers that want a different file.
        let write_db_path = if let Ok(path) = std::env::var("FORGE_WRITE_DB_PATH") {
            std::path::PathBuf::from(path)
        } else {
            base_path.join(".forge.writes.db")
        };
        // Heliosdoctor reports the path the fork actively writes to so the
        // operator can confirm the write/read split is in effect.
        let db_path = write_db_path.clone();
        let binary_stem = forge_config::ConfigReader::binary_prefix().to_string();
        let (updater_repo, updater_binary) = if binary_stem == "helioslite" {
            (
                std::env::var("HELIOSLITE_REPO")
                    .unwrap_or_else(|_| "KooshaPari/heliosLite".to_string()),
                "helioslite".to_string(),
            )
        } else {
            (
                forge_config::DEFAULT_UPDATE_REPO.to_string(),
                "forge".to_string(),
            )
        };
        let config_source = if std::env::var_os("FORGE_CONFIG").is_some() {
            "override-env"
        } else if binary_stem == "helioslite" {
            // base_path is already the resolved directory; classify it by its
            // file name rather than probing subdirectories.
            match base_path.file_name().and_then(|name| name.to_str()) {
                Some(".helioslite") if base_path.exists() => "helioslite",
                Some(".helioslite") => "default",
                Some(".forge") => "legacy-forge",
                _ => "default",
            }
        } else {
            "legacy-forge"
        };
        HeliosdoctorInfo {
            version: forge_config::VERSION.to_string(),
            binary_stem,
            base_path,
            db_path,
            updater_repo,
            updater_binary,
            config_source: config_source.to_string(),
            db_stats: None,
            // Surface both the write and legacy read paths so the operator
            // can verify the fork is using a separate DB. The fields are
            // optional in the domain model so older binaries that don't
            // know about the split still parse this struct cleanly.
            legacy_db_path: if legacy_db_path == write_db_path {
                None
            } else {
                Some(legacy_db_path)
            },
            write_db_path: Some(write_db_path),
        }
    }
}

#[async_trait::async_trait]
impl<F: ProviderRepository + EnvironmentInfra<Config = forge_config::ForgeConfig> + Send + Sync>
    AppConfigService for ForgeAppConfigService<F>
{
    async fn get_session_config(&self) -> Option<ModelConfig> {
        let config = self.infra.get_config().ok()?;
        let session = config.session.as_ref()?;
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

    async fn heliosdoctor(&self) -> anyhow::Result<HeliosdoctorInfo> {
        self.heliosdoctor_verbose(false).await
    }

    async fn heliosdoctor_verbose(&self, verbose: bool) -> anyhow::Result<HeliosdoctorInfo> {
        let mut info = self.heliosdoctor_base_info();
        let db_stats = if verbose {
            match self.infra.database_stats().await {
                Ok(stats) => Some(stats),
                Err(e) => {
                    debug!(error = %e, "heliosdoctor: database_stats failed");
                    None
                }
            }
        } else {
            None
        };
        info.db_stats = db_stats;
        Ok(info)
    }

    async fn heliosdoctor_integrity(&self) -> anyhow::Result<HeliosdoctorInfo> {
        let mut info = self.heliosdoctor_base_info();
        info.db_stats = match self.infra.database_integrity().await {
            Ok(stats) => Some(stats),
            Err(e) => {
                debug!(error = %e, "heliosdoctor: database_integrity failed");
                None
            }
        };
        Ok(info)
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
                            input_modalities: vec![InputModality::Text],
                        }])),
                        custom_headers: None,
                    },
                ],
            }
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
                    }
                }
                Ok(())
            }
        }

        fn get_config(&self) -> anyhow::Result<ForgeConfig> {
            Ok(self.config.lock().unwrap().clone())
        }

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
            std::collections::BTreeMap::new()
        }

        async fn database_stats(&self) -> anyhow::Result<forge_domain::HeliosdoctorDbStats> {
            Ok(forge_domain::HeliosdoctorDbStats::default())
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
}
