use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use config::ConfigBuilder;
use config::builder::DefaultState;

use crate::ForgeConfig;
use crate::legacy::LegacyConfig;

/// Loads all `.env` files found while walking up from the current working
/// directory to the root, with priority given to closer (lower) directories.
/// Executed at most once per process.
static LOAD_DOT_ENV: LazyLock<()> = LazyLock::new(|| {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut paths = vec![];
    let mut current = PathBuf::new();

    for component in cwd.components() {
        current.push(component);
        paths.push(current.clone());
    }

    paths.reverse();

    for path in paths {
        let env_file = path.join(".env");
        if env_file.is_file() {
            dotenvy::from_path(&env_file).ok();
        }
    }
});

/// Caches base-path resolution for the process lifetime.
static BASE_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    ConfigReader::resolve_base_path_for(
        ConfigReader::binary_name(),
        dirs::home_dir().as_deref(),
        None,
    )
    .unwrap_or_else(|error| panic!("unable to resolve configuration root: {error}"))
});

/// Merges [`ForgeConfig`] from layered sources using a builder pattern.
#[derive(Default)]
pub struct ConfigReader {
    builder: ConfigBuilder<DefaultState>,
}

impl ConfigReader {
    /// Returns the path to the legacy JSON config file for the active binary.
    pub fn config_legacy_path() -> PathBuf {
        Self::config_dir().join(".config.json")
    }

    /// Returns the path to the primary TOML config file for the active binary.
    pub fn config_path() -> PathBuf {
        Self::config_path_for(Self::binary_name(), &Self::base_path())
    }

    /// Returns the owned configuration directory for the active binary.
    pub fn config_dir() -> PathBuf {
        let root = Self::base_path();
        if Self::is_helioslite(Self::binary_name()) {
            root.join("config")
        } else {
            root
        }
    }

    /// Returns the runtime cache directory for the active binary.
    pub fn cache_path() -> PathBuf {
        Self::base_path().join("cache")
    }

    /// Returns the runtime logs directory for the active binary.
    pub fn logs_path() -> PathBuf {
        Self::base_path().join("logs")
    }

    /// Returns the runtime lock directory for the active binary.
    pub fn locks_path() -> PathBuf {
        Self::base_path().join("locks")
    }

    /// Returns the runtime session directory for the active binary.
    pub fn sessions_path() -> PathBuf {
        Self::base_path().join("sessions")
    }

    /// Returns the base directory for the active binary's configuration files.
    ///
    /// Resolution order:
    /// 1. For the canonical `helioslite` binary:
    ///    - `HELIOSLITE_HOME`, if set (rejected when it overlaps `~/.forge`).
    ///    - `~/.helioslite` as the isolated default, including when a legacy
    ///      `~/.forge` directory exists. HeliosLite never reads or migrates
    ///      Forge state implicitly.
    /// 2. For the legacy `forge` / `forge-dev` binaries:
    ///    - `FORGE_CONFIG` environment variable, if set.
    ///    - `~/forge` (historical legacy path), if that directory exists.
    ///    - `~/.forge` as the default path.
    pub fn base_path() -> PathBuf {
        BASE_PATH.clone()
    }

    /// Returns the Forge home directory (`~/.forge` or `FORGE_CONFIG` override)
    /// independent of the current binary identity.
    ///
    /// Used by the HeliosLite upstream sync to locate `~/.forge` writes while
    /// running as `helioslite` (whose `base_path()` is `~/.helioslite`).
    pub fn forge_base_path() -> PathBuf {
        Self::resolve_base_path_for("forge", dirs::home_dir().as_deref(), None)
            .unwrap_or_else(|error| panic!("unable to resolve forge home: {error}"))
    }

    /// Whether the current binary is the canonical `helioslite` binary.
    pub fn is_helioslite_binary() -> bool {
        Self::is_helioslite(Self::binary_name())
    }

    pub fn is_helioslite(binary_name: &str) -> bool {
        binary_name.eq_ignore_ascii_case("helioslite")
    }

    fn binary_name() -> &'static str {
        static BINARY_NAME: LazyLock<String> = LazyLock::new(|| {
            std::env::args_os()
                .next()
                .and_then(|arg| {
                    std::path::Path::new(&arg)
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "forge".to_string())
        });
        BINARY_NAME.as_str()
    }

    /// Returns the asset/identity prefix for the running executable:
    /// `helioslite` for the canonical binary, `forge` for the legacy aliases.
    ///
    /// Shared by `forge_main` (updater asset naming) and `forge_services`
    /// (`heliosdoctor` diagnostics) without either depending on the other.
    pub fn binary_prefix() -> &'static str {
        if Self::is_helioslite(Self::binary_name()) {
            "helioslite"
        } else {
            "forge"
        }
    }

    fn resolve_base_path_for(
        binary_name: &str,
        home: Option<&std::path::Path>,
        explicit_home: Option<&std::path::Path>,
    ) -> crate::Result<PathBuf> {
        if Self::is_helioslite(binary_name) {
            if let Some(path) = explicit_home {
                Self::validate_helioslite_home(path, home)?;
                return Ok(path.to_path_buf());
            }
            if let Ok(path) = std::env::var("HELIOSLITE_HOME") {
                let path = PathBuf::from(path);
                Self::validate_helioslite_home(&path, home)?;
                return Ok(path);
            }
            let default = home
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(".helioslite");
            Self::validate_helioslite_home(&default, home)?;
            return Ok(default);
        }

        if let Some(path) = explicit_home {
            return Ok(path.to_path_buf());
        }
        if let Ok(path) = std::env::var("FORGE_CONFIG") {
            return Ok(PathBuf::from(path));
        }

        let base = home.unwrap_or_else(|| std::path::Path::new("."));
        let path = base.join("forge");
        if path.exists() {
            return Ok(path);
        }

        tracing::info!("Using new path");
        Ok(base.join(".forge"))
    }

    fn validate_helioslite_home(
        candidate: &std::path::Path,
        home: Option<&std::path::Path>,
    ) -> crate::Result<()> {
        let Some(home) = home else {
            return Ok(());
        };
        let candidate = Self::canonicalize_for_overlap(candidate)?;
        let forge_root = Self::canonicalize_for_overlap(&home.join(".forge"))?;
        if candidate == forge_root
            || candidate.starts_with(&forge_root)
            || forge_root.starts_with(candidate)
        {
            return Err(crate::Error::Config(config::ConfigError::Message(
                "HELIOSLITE_HOME must not overlap ~/.forge".to_string(),
            )));
        }
        Ok(())
    }

    fn canonicalize_for_overlap(path: &Path) -> crate::Result<PathBuf> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let absolute = Self::lexical_normalize(&absolute);
        let mut ancestor = absolute.clone();
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .ok_or_else(|| {
                    crate::Error::Config(config::ConfigError::Message(format!(
                        "path has no existing ancestor: {}",
                        absolute.display()
                    )))
                })?
                .to_path_buf();
        }
        let canonical_ancestor = ancestor.canonicalize()?;
        let suffix = absolute.strip_prefix(&ancestor).map_err(|_| {
            crate::Error::Config(config::ConfigError::Message(format!(
                "derive non-existent suffix for {} from {}",
                absolute.display(),
                ancestor.display()
            )))
        })?;
        Ok(canonical_ancestor.join(suffix))
    }

    fn lexical_normalize(path: &Path) -> PathBuf {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    normalized.pop();
                }
                component => normalized.push(component.as_os_str()),
            }
        }
        normalized
    }

    fn config_path_for(binary_name: &str, root: &std::path::Path) -> PathBuf {
        if Self::is_helioslite(binary_name) {
            root.join("config/.helioslite.toml")
        } else {
            root.join(".forge.toml")
        }
    }

    /// Adds the provided TOML string as a config source without touching the
    /// filesystem.
    pub fn read_toml(mut self, contents: &str) -> Self {
        self.builder = self
            .builder
            .add_source(config::File::from_str(contents, config::FileFormat::Toml));

        self
    }

    /// Adds the embedded default config (`../.forge.toml`) as a source.
    pub fn read_defaults(self) -> Self {
        let defaults = include_str!("../.forge.toml");

        self.read_toml(defaults)
    }

    /// Adds `FORGE_`-prefixed environment variables as a config source.
    pub fn read_env(mut self) -> Self {
        self.builder = self.builder.add_source(
            config::Environment::with_prefix("FORGE")
                .prefix_separator("_")
                .separator("__")
                .try_parsing(true)
                .list_separator(",")
                .with_list_parse_key("retry.status_codes")
                .with_list_parse_key("http.root_cert_paths"),
        );

        self
    }

    /// Builds and deserializes all accumulated sources into a [`ForgeConfig`].
    ///
    /// Triggers `.env` file loading (at most once per process) by walking up
    /// the directory tree from the current working directory, with closer
    /// directories taking priority.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be built or deserialized.
    pub fn build(self) -> crate::Result<ForgeConfig> {
        *LOAD_DOT_ENV;
        let config = self.builder.build()?;
        Ok(config.try_deserialize::<ForgeConfig>()?)
    }

    /// Adds `~/.forge/.forge.toml` as a config source, silently skipping if
    /// absent.
    pub fn read_global(mut self) -> Self {
        let path = Self::config_path();
        self.builder = self
            .builder
            .add_source(config::File::from(path).required(false));
        self
    }

    /// Reads `~/.forge/.config.json` (legacy format) and adds it as a source,
    /// silently skipping errors.
    pub fn read_legacy(self) -> Self {
        let content = LegacyConfig::read(&Self::config_legacy_path());
        if let Ok(content) = content {
            self.read_toml(&content)
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::ModelConfig;

    /// Serializes tests that mutate environment variables to prevent races.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// Holds env vars set for a test's duration and removes them on drop, while
    /// holding [`ENV_MUTEX`].
    struct EnvGuard {
        keys: Vec<&'static str>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        /// Acquires [`ENV_MUTEX`], sets each `(key, value)` pair in the
        /// environment, and removes each key in `remove` if present. All
        /// set keys are cleaned up on drop.
        #[must_use]
        fn set(pairs: &[(&'static str, &str)]) -> Self {
            Self::set_and_remove(pairs, &[])
        }

        /// Like [`set`] but also removes the listed keys before the test runs.
        #[must_use]
        fn set_and_remove(pairs: &[(&'static str, &str)], remove: &[&'static str]) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let keys = pairs.iter().map(|(k, _)| *k).collect();
            for key in remove {
                unsafe { std::env::remove_var(key) };
            }
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
    fn test_base_path_uses_forge_config_env_var() {
        let _guard = EnvGuard::set(&[("FORGE_CONFIG", "/custom/forge/dir")]);
        let actual =
            ConfigReader::resolve_base_path_for("forge", dirs::home_dir().as_deref(), None)
                .unwrap();
        let expected = PathBuf::from("/custom/forge/dir");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_base_path_falls_back_to_home_dir_when_env_var_absent() {
        // Hold the env mutex and ensure FORGE_CONFIG is absent so this test
        // cannot race with test_base_path_uses_forge_config_env_var.
        let _guard = EnvGuard::set_and_remove(&[], &["FORGE_CONFIG"]);

        let actual =
            ConfigReader::resolve_base_path_for("forge", dirs::home_dir().as_deref(), None)
                .unwrap();
        // Without FORGE_CONFIG set the path must be either "forge" (legacy,
        // preferred when ~/forge exists) or ".forge" (default new path).
        let name = actual.file_name().unwrap();
        assert!(
            name == "forge" || name == ".forge",
            "Expected base_path to end with 'forge' or '.forge', got: {:?}",
            name
        );
    }

    #[test]
    fn helioslite_uses_owned_root_and_config_subdirectory() {
        let _guard = EnvGuard::set_and_remove(&[], &["HELIOSLITE_HOME"]);
        let home = PathBuf::from("/tmp/helios-home");
        let root = ConfigReader::resolve_base_path_for("helioslite", Some(&home), None).unwrap();
        assert_eq!(root, home.join(".helioslite"));
        assert_eq!(
            ConfigReader::config_path_for("helioslite", &root),
            root.join("config/.helioslite.toml")
        );
    }
    #[test]
    fn helioslite_explicit_home_is_independent_from_forge_config() {
        let home = PathBuf::from("/tmp/helios-home");
        let explicit = PathBuf::from("/tmp/explicit-helioslite");
        let root = ConfigReader::resolve_base_path_for("helioslite", Some(&home), Some(&explicit));
        assert_eq!(root.unwrap(), explicit);
    }
    #[test]
    fn forge_keeps_legacy_root_and_config_layout() {
        let home = PathBuf::from("/tmp/forge-home");
        let explicit = PathBuf::from("/tmp/explicit-forge");
        let root =
            ConfigReader::resolve_base_path_for("forge", Some(&home), Some(&explicit)).unwrap();
        assert_eq!(root, explicit);
        assert_eq!(
            ConfigReader::config_path_for("forge", &root),
            explicit.join(".forge.toml")
        );
    }
    #[test]
    fn helioslite_detection_is_case_insensitive() {
        let _guard = EnvGuard::set_and_remove(&[], &["HELIOSLITE_HOME"]);
        let home = PathBuf::from("/tmp/helios-home");
        let root = ConfigReader::resolve_base_path_for("HeLiOsLiTe", Some(&home), None).unwrap();
        assert_eq!(root, home.join(".helioslite"));
        assert_eq!(
            ConfigReader::config_path_for("HELIOSLITE", &root),
            root.join("config/.helioslite.toml")
        );
    }
    #[test]
    fn helioslite_home_rejects_overlap_with_forge_root() {
        let home = PathBuf::from("/tmp/helios-home");
        for candidate in [
            home.join(".forge"),
            home.join(".forge/sessions"),
            home.clone(),
        ] {
            let error =
                ConfigReader::resolve_base_path_for("helioslite", Some(&home), Some(&candidate))
                    .expect_err("overlapping HeliosLite root must be rejected");
            assert!(error.to_string().contains("must not overlap ~/.forge"));
        }
    }
    #[test]
    fn helioslite_env_home_rejects_overlap_with_forge_root() {
        let home = PathBuf::from("/tmp/helios-home");
        let forge_root = home.join(".forge");
        let value = forge_root.to_str().unwrap();
        let _guard = EnvGuard::set(&[("HELIOSLITE_HOME", value)]);
        let error = ConfigReader::resolve_base_path_for("HeLiOsLiTe", Some(&home), None)
            .expect_err("overlapping HELIOSLITE_HOME must be rejected");
        assert!(error.to_string().contains("must not overlap ~/.forge"));
    }

    #[test]
    fn test_base_path_canonical_binary_does_not_adopt_legacy_forge_dir() {
        let _guard = EnvGuard::set_and_remove(&[], &["FORGE_CONFIG", "HELIOSLITE_HOME"]);
        let home = std::env::temp_dir().join(format!("hl-gate5-legacy-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".forge")).unwrap();
        let actual = ConfigReader::resolve_base_path_for("helioslite", Some(&home), None).unwrap();
        std::fs::remove_dir_all(&home).ok();
        assert_eq!(actual, home.join(".helioslite"));
    }

    #[test]
    fn helioslite_default_root_remains_independent_when_forge_exists() {
        let _guard = EnvGuard::set_and_remove(&[], &["FORGE_CONFIG", "HELIOSLITE_HOME"]);
        let home = std::env::temp_dir().join(format!("hl-default-overlap-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".forge")).unwrap();

        let actual = ConfigReader::resolve_base_path_for("helioslite", Some(&home), None);

        std::fs::remove_dir_all(&home).ok();
        assert_eq!(
            actual.unwrap(),
            home.join(".helioslite"),
            "HeliosLite must not use the Forge root by default"
        );
    }

    #[test]
    fn test_base_path_canonical_binary_keeps_owned_root_when_both_exist() {
        let _guard = EnvGuard::set_and_remove(&[], &["FORGE_CONFIG", "HELIOSLITE_HOME"]);
        let home = std::env::temp_dir().join(format!("hl-gate5-canon-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".helioslite")).unwrap();
        std::fs::create_dir_all(home.join(".forge")).unwrap();
        let actual = ConfigReader::resolve_base_path_for("helioslite", Some(&home), None).unwrap();
        std::fs::remove_dir_all(&home).ok();
        assert_eq!(actual, home.join(".helioslite"));
    }

    #[test]
    fn test_base_path_canonical_binary_uses_helioslite_after_legacy_removed() {
        // After ~/.forge is moved away (migration), the canonical dir wins.
        let _guard = EnvGuard::set_and_remove(&[], &["FORGE_CONFIG", "HELIOSLITE_HOME"]);
        let home = std::env::temp_dir().join(format!("hl-gate5-migrated-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".helioslite")).unwrap();
        let actual = ConfigReader::resolve_base_path_for("helioslite", Some(&home), None).unwrap();
        std::fs::remove_dir_all(&home).ok();
        assert_eq!(actual, home.join(".helioslite"));
    }

    #[test]
    fn test_base_path_legacy_binary_defaults_to_dot_forge() {
        let _guard = EnvGuard::set_and_remove(&[], &["FORGE_CONFIG"]);
        let home = PathBuf::from("/home/nonexistent-user");
        let actual = ConfigReader::resolve_base_path_for("forge", Some(&home), None).unwrap();
        assert_eq!(actual, home.join(".forge"));
    }

    #[test]
    fn test_read_parses_without_error() {
        let actual = ConfigReader::default().read_defaults().build();
        assert!(actual.is_ok(), "read() failed: {:?}", actual.err());
    }

    #[test]
    fn test_legacy_layer_does_not_overwrite_defaults() {
        // Simulate what `read_legacy` does: serialize a ForgeConfig that only
        // carries session/commit/suggest (all other fields are None) and layer
        // it on top of the embedded defaults. The default values must survive.
        let legacy = ForgeConfig {
            session: Some(ModelConfig {
                provider_id: "anthropic".to_string(),
                model_id: "claude-3".to_string(),
            }),
            ..Default::default()
        };
        let legacy_toml = toml_edit::ser::to_string_pretty(&legacy).unwrap();

        let actual = ConfigReader::default()
            // Read legacy first and then defaults
            .read_toml(&legacy_toml)
            .read_defaults()
            .build()
            .unwrap();

        // Session should come from the legacy layer
        assert_eq!(
            actual.session,
            Some(ModelConfig {
                provider_id: "anthropic".to_string(),
                model_id: "claude-3".to_string(),
            })
        );

        // Default values from .forge.toml must be retained, not reset to zero
        assert_eq!(actual.max_parallel_file_reads, 64);
        assert_eq!(actual.max_read_lines, 2000);
        assert_eq!(actual.tool_timeout_secs, 300);
        assert_eq!(actual.max_search_lines, 1000);
        assert_eq!(actual.tool_supported, true);
    }

    #[test]
    fn test_read_session_from_env_vars() {
        let _guard = EnvGuard::set(&[
            ("FORGE_SESSION__PROVIDER_ID", "fake-provider"),
            ("FORGE_SESSION__MODEL_ID", "fake-model"),
        ]);

        let actual = ConfigReader::default()
            .read_defaults()
            .read_env()
            .build()
            .unwrap();

        let expected = Some(ModelConfig {
            provider_id: "fake-provider".to_string(),
            model_id: "fake-model".to_string(),
        });
        assert_eq!(actual.session, expected);
    }

    #[test]
    fn test_use_forge_committer_defaults_to_true() {
        let actual = ConfigReader::default().read_defaults().build().unwrap();

        assert_eq!(actual.use_forge_committer, true);
    }

    #[test]
    fn test_use_forge_committer_can_be_disabled() {
        let toml = "use_forge_committer = false\n";

        let actual = ConfigReader::default()
            .read_defaults()
            .read_toml(toml)
            .build()
            .unwrap();

        assert_eq!(actual.use_forge_committer, false);
    }
}
