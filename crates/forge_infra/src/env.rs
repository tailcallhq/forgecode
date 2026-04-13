use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use forge_app::EnvironmentInfra;
use forge_config::{ConfigReader, ForgeConfig, ModelConfig};
use forge_domain::{ConfigOperation, Environment};
use tracing::debug;

/// Builds a [`forge_domain::Environment`] from runtime context only.
///
/// Only the five fields that cannot be sourced from [`ForgeConfig`] are set
/// here: `os`, `cwd`, `home`, `shell`, and `base_path`. All configuration
/// values are now accessed through `EnvironmentInfra::get_config()`.
pub fn to_environment(cwd: PathBuf) -> Environment {
    Environment {
        os: std::env::consts::OS.to_string(),
        cwd,
        home: dirs::home_dir(),
        shell: if cfg!(target_os = "windows") {
            std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
        },
        base_path: ConfigReader::base_path(),
    }
}

/// Applies a single [`ConfigOperation`] directly to a [`ForgeConfig`].
///
/// Used by [`ForgeEnvironmentInfra::update_environment`] to mutate the
/// persisted config without an intermediate `Environment` round-trip.
fn apply_config_op(fc: &mut ForgeConfig, op: ConfigOperation) {
    match op {
        ConfigOperation::SetSessionConfig(mc) => {
            let pid_str = mc.provider.as_ref().to_string();
            let mid_str = mc.model.to_string();
            fc.session = Some(ModelConfig { provider_id: pid_str, model_id: mid_str });
        }
        ConfigOperation::SetCommitConfig(mc) => {
            fc.commit = mc.map(|m| ModelConfig {
                provider_id: m.provider.as_ref().to_string(),
                model_id: m.model.to_string(),
            });
        }
        ConfigOperation::SetSuggestConfig(mc) => {
            fc.suggest = Some(ModelConfig {
                provider_id: mc.provider.as_ref().to_string(),
                model_id: mc.model.to_string(),
            });
        }
        ConfigOperation::SetReasoningEffort(effort) => {
            let config_effort = match effort {
                forge_domain::Effort::None => forge_config::Effort::None,
                forge_domain::Effort::Minimal => forge_config::Effort::Minimal,
                forge_domain::Effort::Low => forge_config::Effort::Low,
                forge_domain::Effort::Medium => forge_config::Effort::Medium,
                forge_domain::Effort::High => forge_config::Effort::High,
                forge_domain::Effort::XHigh => forge_config::Effort::XHigh,
                forge_domain::Effort::Max => forge_config::Effort::Max,
            };
            let reasoning = fc
                .reasoning
                .get_or_insert_with(forge_config::ReasoningConfig::default);
            reasoning.effort = Some(config_effort);
        }
    }
}

/// Infrastructure implementation for managing application configuration with
/// caching support.
///
/// Uses [`ForgeConfig::read`] and [`ForgeConfig::write`] for all file I/O and
/// maintains an in-memory cache to reduce disk access. Also handles
/// environment variable discovery via `.env` files and OS APIs.
pub struct ForgeEnvironmentInfra {
    cwd: PathBuf,
    cache: Arc<std::sync::Mutex<Option<ForgeConfig>>>,
}

impl ForgeEnvironmentInfra {
    /// Creates a new [`ForgeEnvironmentInfra`] with the given pre-read config.
    ///
    /// The cache is pre-seeded with `config` so no disk I/O occurs on the
    /// first [`EnvironmentInfra::get_config`] call.
    ///
    /// # Arguments
    /// * `cwd` - The working directory path; used to resolve `.env` files
    /// * `config` - The pre-read [`ForgeConfig`] to seed the in-memory cache
    pub fn new(cwd: PathBuf, config: ForgeConfig) -> Self {
        Self { cwd, cache: Arc::new(std::sync::Mutex::new(Some(config))) }
    }

    /// Returns the cached [`ForgeConfig`], re-reading from disk if the cache
    /// has been invalidated by [`Self::update_environment`].
    ///
    /// # Errors
    ///
    /// Returns an error if the cache is empty and the disk read fails.
    pub fn cached_config(&self) -> anyhow::Result<ForgeConfig> {
        let mut cache = self.cache.lock().expect("cache mutex poisoned");
        if let Some(ref config) = *cache {
            Ok(config.clone())
        } else {
            let config = ConfigReader::default()
                .read_defaults()
                .read_global()
                .read_env()
                .build()?;
            *cache = Some(config.clone());
            Ok(config)
        }
    }
}

impl EnvironmentInfra for ForgeEnvironmentInfra {
    type Config = ForgeConfig;

    fn get_env_var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn get_env_vars(&self) -> BTreeMap<String, String> {
        std::env::vars().collect()
    }

    fn get_environment(&self) -> Environment {
        to_environment(self.cwd.clone())
    }

    fn get_config(&self) -> anyhow::Result<ForgeConfig> {
        self.cached_config()
    }

    async fn update_environment(&self, ops: Vec<ConfigOperation>) -> anyhow::Result<()> {
        // Load the global config (with defaults applied) for the update round-trip
        let mut fc = ConfigReader::default()
            .read_defaults()
            .read_global()
            .build()?;

        debug!(config = ?fc, ?ops, "applying app config operations");

        for op in ops {
            apply_config_op(&mut fc, op);
        }

        fc.write()?;
        debug!(config = ?fc, "written .forge.toml");

        // Reset cache so next get_config() re-reads the updated values from disk
        *self.cache.lock().expect("cache mutex poisoned") = None;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};

    use forge_config::ForgeConfig;
    use pretty_assertions::assert_eq;

    use super::*;

    /// Serializes tests that mutate environment variables to prevent races.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// Holds env vars set for a test's duration and removes them on drop,
    /// while holding [`ENV_MUTEX`].
    struct EnvGuard {
        keys: Vec<&'static str>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        #[must_use]
        fn set(pairs: &[(&'static str, &str)]) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let keys = pairs.iter().map(|(k, _)| *k).collect();
            for (key, value) in pairs {
                unsafe { std::env::set_var(key, value) };
            }
            Self { keys, _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for key in &self.keys {
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    #[test]
    fn test_to_environment_sets_cwd() {
        let fixture_cwd = PathBuf::from("/test/cwd");
        let actual = to_environment(fixture_cwd.clone());
        assert_eq!(actual.cwd, fixture_cwd);
    }

    #[test]
    fn test_to_environment_uses_forge_config_env_var() {
        let _guard = EnvGuard::set(&[("FORGE_CONFIG", "/custom/config/dir")]);
        let actual = to_environment(PathBuf::from("/any/cwd"));
        let expected = PathBuf::from("/custom/config/dir");
        assert_eq!(actual.base_path, expected);
    }

    #[test]
    fn test_to_environment_falls_back_to_home_dir_when_env_var_absent() {
        let actual = to_environment(PathBuf::from("/any/cwd"));
        // Without FORGE_CONFIG the base_path must be either ".forge" (new default)
        // or "forge" (legacy fallback when ~/forge exists on this machine).
        let name = actual.base_path.file_name().unwrap();
        assert!(
            name == ".forge" || name == "forge",
            "Expected base_path to end with '.forge' or 'forge', got: {:?}",
            name
        );
    }

    #[test]
    fn test_apply_config_op_set_model() {
        use forge_domain::{ModelConfig as DomainModelConfig, ModelId, ProviderId};

        let mut fixture = ForgeConfig::default();
        apply_config_op(
            &mut fixture,
            ConfigOperation::SetSessionConfig(DomainModelConfig::new(
                ProviderId::ANTHROPIC,
                ModelId::new("claude-3-5-sonnet"),
            )),
        );

        let actual_provider = fixture.session.as_ref().map(|s| s.provider_id.as_str());
        let actual_model = fixture.session.as_ref().map(|s| s.model_id.as_str());

        assert_eq!(actual_provider, Some("anthropic"));
        assert_eq!(actual_model, Some("claude-3-5-sonnet"));
    }

    #[test]
    fn test_apply_config_op_set_session_config_replaces_existing() {
        use forge_config::ModelConfig as ForgeCfgModelConfig;
        use forge_domain::{ModelConfig as DomainModelConfig, ModelId, ProviderId};

        let mut fixture = ForgeConfig {
            session: Some(ForgeCfgModelConfig {
                provider_id: "openai".to_string(),
                model_id: "gpt-4".to_string(),
            }),
            ..Default::default()
        };

        apply_config_op(
            &mut fixture,
            ConfigOperation::SetSessionConfig(DomainModelConfig::new(
                ProviderId::ANTHROPIC,
                ModelId::new("claude-3-5-sonnet-20241022"),
            )),
        );

        let actual_provider = fixture.session.as_ref().map(|s| s.provider_id.as_str());
        let actual_model = fixture.session.as_ref().map(|s| s.model_id.as_str());

        assert_eq!(actual_provider, Some("anthropic"));
        assert_eq!(actual_model, Some("claude-3-5-sonnet-20241022"));
    }

    #[test]
    fn test_apply_config_op_set_session_config_creates_new_session() {
        use forge_domain::{ModelConfig as DomainModelConfig, ModelId, ProviderId};

        let mut fixture = ForgeConfig::default();

        apply_config_op(
            &mut fixture,
            ConfigOperation::SetSessionConfig(DomainModelConfig::new(
                ProviderId::ANTHROPIC,
                ModelId::new("claude-3-5-sonnet-20241022"),
            )),
        );

        let actual_provider = fixture.session.as_ref().map(|s| s.provider_id.as_str());
        let actual_model = fixture.session.as_ref().map(|s| s.model_id.as_str());

        assert_eq!(actual_provider, Some("anthropic"));
        assert_eq!(actual_model, Some("claude-3-5-sonnet-20241022"));
    }
}
