use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use forge_app::EnvironmentInfra;
use forge_config::{ConfigReader, ForgeConfig, ModelConfig};
use forge_domain::{ConfigOperation, Environment};
use tokio::task::JoinHandle;
use tracing::debug;

/// Interval in seconds between sudo keepalive refreshes.
const SUDO_KEEPALIVE_INTERVAL_SECS: u64 = 30; // 30 seconds

/// Spawns a platform-specific child process that inhibits system sleep.
///
/// - **Linux**: uses `systemd-inhibit` to block the `sleep` and `idle` modes.
/// - **macOS**: uses `caffeinate -s` to prevent sleep while on AC power.
///
/// On unsupported platforms the child exits immediately — the caller treats
/// this as a no-op.
fn spawn_inhibit_process() -> tokio::process::Child {
    let mut cmd = if cfg!(target_os = "linux") {
        let mut c = tokio::process::Command::new("systemd-inhibit");
        c.args([
            "--what",
            "sleep:idle",
            "--who",
            "forge",
            "--why",
            "forge is running",
            "--mode",
            "block",
            "sleep",
            "infinity",
        ]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = tokio::process::Command::new("caffeinate");
        c.arg("-s");
        c
    } else {
        // Unsupported platform — spawn a process that exits immediately
        let c = tokio::process::Command::new("true");
        c
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn().unwrap_or_else(|_| {
        // If the inhibit command is not available, fall back to a no-op.
        tokio::process::Command::new("true")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("`true` should always be spawnable")
    })
}

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
        ConfigOperation::SetShellConfig(mc) => {
            fc.shell = Some(ModelConfig {
                provider_id: mc.provider.as_ref().to_string(),
                model_id: mc.model.to_string(),
            });
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
        ConfigOperation::SetSudo(enabled) => {
            fc.sudo = enabled;
        }
        ConfigOperation::SetPreventSleep(enabled) => {
            fc.prevent_sleep = enabled;
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
    /// Shared flag mirrored into the command executor so it can prefix
    /// commands with `sudo` without re-reading config from disk.
    sudo: Arc<AtomicBool>,
    /// Handle for the background task that periodically refreshes the sudo
    /// timestamp so it does not expire during a long session.
    keepalive: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
    /// Handle for the background task that prevents system sleep/hibernate.
    sleep_inhibit: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
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
        // Always start with sudo disabled regardless of what was persisted
        // in the config file. Sudo mode is session-scoped (in-memory only)
        // and must be explicitly activated with `:su` each time forge starts.
        let sudo = Arc::new(AtomicBool::new(false));
        Self {
            cwd,
            cache: Arc::new(std::sync::Mutex::new(Some(config))),
            sudo,
            keepalive: Arc::new(tokio::sync::Mutex::new(None)),
            sleep_inhibit: Arc::new(tokio::sync::Mutex::new(None)),
        }
        // Note: sleep inhibition is started lazily via `start_sleep_inhibition`
        // because `new` is not async.
    }

    /// Returns the shared `sudo` flag so the command executor can observe it.
    pub fn sudo_flag(&self) -> Arc<AtomicBool> {
        self.sudo.clone()
    }

    /// Returns the cached [`ForgeConfig`], re-reading from disk if the cache
    /// has been invalidated by [`Self::update_environment`].
    ///
    /// # Errors
    ///
    /// Returns an error if the cache is empty and the disk read fails.
    pub fn cached_config(&self) -> anyhow::Result<ForgeConfig> {
        let mut cache = self.cache.lock().expect("cache mutex poisoned");
        let mut config = if let Some(ref config) = *cache {
            config.clone()
        } else {
            let config = ConfigReader::default()
                .read_defaults()
                .read_global()
                .read_env()
                .build()?;
            *cache = Some(config.clone());
            config
        };
        // Override the sudo field with the in-memory flag so that
        // get_config always reflects the session-scoped sudo state
        // rather than whatever was persisted to disk.
        config.sudo = self.sudo.load(Ordering::Relaxed);
        Ok(config)
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
            if let ConfigOperation::SetSudo(enabled) = op {
                self.sudo.store(enabled, Ordering::Relaxed);
                self.refresh_keepalive(enabled).await;
                // Sudo is session-scoped: update the in-memory flag only,
                // do NOT persist to disk.
                continue;
            }
            if let ConfigOperation::SetPreventSleep(enabled) = op {
                self.refresh_sleep_inhibition(enabled).await;
                // Persist prevent_sleep to disk (not session-scoped).
            }
            apply_config_op(&mut fc, op);
        }

        fc.write()?;
        debug!(config = ?fc, "written .forge.toml");

        // Reset cache so next get_config() re-reads the updated values from disk
        *self.cache.lock().expect("cache mutex poisoned") = None;

        Ok(())
    }
}

impl ForgeEnvironmentInfra {
    /// Returns the sudo keepalive interval in seconds.
    ///
    /// A shorter interval keeps credentials warm for the lifetime of the
    /// session without requiring an interactive password prompt.
    pub fn sudo_keepalive_interval_secs() -> u64 {
        SUDO_KEEPALIVE_INTERVAL_SECS
    }

    /// Starts or stops the sudo keepalive background task.
    ///
    /// When `enabled` is `true`, spawns a background task that refreshes the
    /// sudo timestamp immediately and then revalidates often enough that it
    /// stays warm for the duration of the session. When `enabled` is `false`,
    /// aborts any running keepalive task.
    pub(crate) async fn refresh_keepalive(&self, enabled: bool) {
        let mut guard = self.keepalive.lock().await;
        // Abort any existing keepalive task
        if let Some(handle) = guard.take() {
            handle.abort();
        }
        if enabled {
            let handle = tokio::spawn(async {
                loop {
                    // Refresh immediately when sudo mode is enabled, then
                    // continue to revalidate often enough that the timestamp
                    // stays warm for the lifetime of the session.
                    let _ = std::process::Command::new("sudo")
                        .arg("-n")
                        .arg("-v")
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status();

                    tokio::time::sleep(std::time::Duration::from_secs(
                        SUDO_KEEPALIVE_INTERVAL_SECS,
                    ))
                    .await;
                }
            });
            *guard = Some(handle);
        }
    }

    /// Starts or stops the system sleep/hibernate inhibition.
    ///
    /// When `enabled` is `true`, spawns a long-running child process that
    /// holds a system-level sleep inhibition lock. The exact command depends
    /// on the current platform:
    ///
    /// - **Linux**: `systemd-inhibit --what=sleep --who=forge --why="forge is running" --mode=block sleep infinity`
    /// - **macOS**: `caffeinate -s`
    ///
    /// When `enabled` is `false`, aborts any running inhibition task (which
    /// drops the child process and releases the lock).
    pub(crate) async fn refresh_sleep_inhibition(&self, enabled: bool) {
        let mut guard = self.sleep_inhibit.lock().await;
        if let Some(handle) = guard.take() {
            handle.abort();
        }
        if enabled {
            let handle = tokio::spawn(async {
                let mut child = spawn_inhibit_process();
                // Wait for the child process indefinitely. If it exits
                // (e.g. systemd-inhibit not available), simply return.
                let _ = child.wait().await;
            });
            *guard = Some(handle);
        }
    }

    /// Starts sleep inhibition if the config has `prevent_sleep = true`.
    ///
    /// Call this once after construction (from an async context).
    pub async fn start_sleep_inhibition(&self) {
        let enabled = self
            .cached_config()
            .map(|c| c.prevent_sleep)
            .unwrap_or(true);
        self.refresh_sleep_inhibition(enabled).await;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use forge_config::ForgeConfig;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_to_environment_sets_cwd() {
        let fixture_cwd = PathBuf::from("/test/cwd");
        let actual = to_environment(fixture_cwd.clone());
        assert_eq!(actual.cwd, fixture_cwd);
    }

    #[test]
    fn test_to_environment_base_path_is_stable_after_env_var_change() {
        let fixture_cwd = PathBuf::from("/any/cwd");
        let expected = to_environment(fixture_cwd.clone()).base_path;

        let previous = std::env::var("FORGE_CONFIG").ok();
        unsafe { std::env::set_var("FORGE_CONFIG", "/custom/config/dir") };

        let actual = to_environment(fixture_cwd).base_path;

        if let Some(value) = previous {
            unsafe { std::env::set_var("FORGE_CONFIG", value) };
        } else {
            unsafe { std::env::remove_var("FORGE_CONFIG") };
        }

        assert_eq!(actual, expected);
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
