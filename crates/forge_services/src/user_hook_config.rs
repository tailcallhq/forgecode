use std::sync::Arc;

use forge_app::EnvironmentInfra;
use forge_config::UserHookConfig;

/// Loads user hook configuration from `.forge.toml` via the config pipeline.
///
/// Hook configuration is read from the `[hooks]` section of the user's
/// `.forge.toml` file, automatically merged with defaults by the
/// `ConfigReader` layered config system.
pub struct ForgeUserHookConfigService<F>(Arc<F>);

impl<F> ForgeUserHookConfigService<F> {
    /// Creates a new service with the given infrastructure dependency.
    pub fn new(infra: Arc<F>) -> Self {
        Self(infra)
    }
}

#[async_trait::async_trait]
impl<F: EnvironmentInfra<Config = forge_config::ForgeConfig>> forge_app::UserHookConfigService
    for ForgeUserHookConfigService<F>
{
    async fn get_user_hook_config(&self) -> anyhow::Result<UserHookConfig> {
        Ok(self.0.get_config()?.hooks.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fake::Fake;
    use forge_app::UserHookConfigService;
    use forge_config::UserHookEventName;
    use pretty_assertions::assert_eq;

    use super::*;

    #[tokio::test]
    async fn test_get_user_hook_config_returns_hooks_from_config() {
        let hook_json = r#"{
            "PreToolUse": [
                { "matcher": "Bash", "hooks": [{ "type": "command", "command": "check.sh" }] }
            ]
        }"#;
        let hooks: forge_config::UserHookConfig = serde_json::from_str(hook_json).unwrap();
        let config = forge_config::ForgeConfig { hooks: Some(hooks), ..Default::default() };
        let service = fixture(config);

        let actual = service.get_user_hook_config().await.unwrap();

        assert!(!actual.is_empty());
        assert_eq!(actual.get_groups(&UserHookEventName::PreToolUse).len(), 1);
    }

    #[tokio::test]
    async fn test_get_user_hook_config_returns_empty_when_no_hooks() {
        let config = forge_config::ForgeConfig::default();
        let service = fixture(config);

        let actual = service.get_user_hook_config().await.unwrap();

        assert!(actual.is_empty());
    }

    // --- Test helpers ---

    fn fixture(config: forge_config::ForgeConfig) -> ForgeUserHookConfigService<TestInfra> {
        ForgeUserHookConfigService::new(Arc::new(TestInfra { config }))
    }

    struct TestInfra {
        config: forge_config::ForgeConfig,
    }

    impl EnvironmentInfra for TestInfra {
        type Config = forge_config::ForgeConfig;

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
            Default::default()
        }

        fn get_environment(&self) -> forge_domain::Environment {
            let mut env: forge_domain::Environment = fake::Faker.fake();
            env.home = Some(PathBuf::from("/nonexistent/home"));
            env.cwd = PathBuf::from("/nonexistent/project");
            env
        }

        fn get_config(&self) -> anyhow::Result<Self::Config> {
            Ok(self.config.clone())
        }

        async fn update_environment(
            &self,
            _ops: Vec<forge_domain::ConfigOperation>,
        ) -> anyhow::Result<()> {
            unimplemented!("not needed for tests")
        }
    }
}
