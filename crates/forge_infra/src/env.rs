use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use forge_app::EnvironmentInfra;
use forge_config::{ConfigReader, ForgeConfig, ModelConfig};
use forge_domain::{ConfigOperation, Environment, HeliosdoctorDbStats};
use tracing::debug;

/// Returns the absolute path to the file the fork writes conversations into.
///
/// Mirrors [`forge_domain::Environment::write_database_path`] but resolves
/// `base_path` from [`ConfigReader`] so callers that don't have a full
/// `Environment` (e.g. diagnostic helpers) can still locate the file.
fn database_write_path() -> PathBuf {
    if let Ok(path) = std::env::var("FORGE_WRITE_DB_PATH") {
        return PathBuf::from(path);
    }
    // Split-DB default: writes go to `.forge.writes.db` while the read side
    // unions in the legacy `.forge.db` via the `conversations_all` TEMP VIEW.
    // This mirrors `forge_domain::Environment::write_database_path()` so the
    // infra stats helper and the pool always agree on the primary file.
    ConfigReader::base_path().join(".forge.writes.db")
}

/// Returns the path to the legacy `.forge.db` file (or whatever the user
/// pointed `FORGE_LEGACY_DB_PATH` at). Returns `None` when no legacy file
/// is configured.
fn database_legacy_read_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("FORGE_LEGACY_DB_PATH") {
        return Some(PathBuf::from(path));
    }
    Some(ConfigReader::base_path().join(".forge.db"))
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

        fc.write(None)?;
        debug!(config = ?fc, "written .forge.toml");

        // Reset cache so next get_config() re-reads the updated values from disk
        *self.cache.lock().expect("cache mutex poisoned") = None;

        Ok(())
    }

    fn database_stats(
        &self,
    ) -> impl std::future::Future<Output = anyhow::Result<HeliosdoctorDbStats>> + Send {
        let stats = compute_database_stats();
        async move { Ok(stats) }
    }

    fn database_integrity(
        &self,
    ) -> impl std::future::Future<Output = anyhow::Result<HeliosdoctorDbStats>> + Send {
        let stats = compute_database_integrity();
        async move { Ok(stats) }
    }
}

/// Runs `PRAGMA integrity_check` on the write DB and (when split-DB is
/// active) the legacy read DB, without the COUNT queries that
/// [`compute_database_stats`] performs. Surfaces the per-DB results on
/// `integrity_check` ("ok" when every checked DB is healthy) and records the
/// paths checked on `write_db_path` / `legacy_db_path`.
fn compute_database_integrity() -> HeliosdoctorDbStats {
    use diesel::sql_types::Text;
    use diesel::sqlite::SqliteConnection;
    use diesel::{Connection, RunQueryDsl};

    let write_path = database_write_path();
    let legacy_path = database_legacy_read_path();

    let mut stats = HeliosdoctorDbStats {
        write_db_path: Some(write_path.to_string_lossy().to_string()),
        legacy_db_path: legacy_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        ..Default::default()
    };

    #[derive(diesel::QueryableByName)]
    struct IntegrityRow {
        #[diesel(sql_type = Text)]
        integrity_check: String,
    }
    let check = |conn: &mut diesel::sqlite::SqliteConnection| -> String {
        diesel::sql_query("PRAGMA integrity_check")
            .get_result::<IntegrityRow>(conn)
            .map(|r| r.integrity_check)
            .unwrap_or_else(|_| "unknown".to_string())
    };

    let mut checks: Vec<(String, String)> = Vec::new();
    match SqliteConnection::establish(write_path.to_string_lossy().as_ref()) {
        Ok(mut conn) => checks.push(("primary".to_string(), check(&mut conn))),
        Err(err) => {
            stats.error = Some(format!("open write db: {err}"));
            stats.integrity_check = "error".to_string();
            return stats;
        }
    }

    // The legacy DB is only part of the split when it resolves to a different
    // file than the write DB. It is checked separately (no ATTACH needed for
    // a standalone PRAGMA), read-only by construction.
    if let Some(legacy) = legacy_path.as_ref()
        && legacy != &write_path
        && legacy.exists()
    {
        match SqliteConnection::establish(legacy.to_string_lossy().as_ref()) {
            Ok(mut legacy_conn) => {
                stats.legacy_attached = Some(true);
                checks.push(("legacy".to_string(), check(&mut legacy_conn)));
            }
            Err(err) => {
                stats.legacy_attached = Some(false);
                stats.error = Some(format!("open legacy db: {err}"));
                checks.push(("legacy".to_string(), "error".to_string()));
            }
        }
    }

    stats.integrity_check = if checks.iter().all(|(_, c)| c == "ok") {
        "ok".to_string()
    } else {
        checks
            .iter()
            .map(|(name, c)| format!("{name}: {c}"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    stats
}

/// Computes live database row counts from the write DB (and the legacy DB if
/// it still exists and is reachable).
///
/// ATTACHes the legacy DB at `legacy_path` to the write DB at `write_path`,
/// then queries both via a UNION ALL so legacy rows are still visible from
/// `/conversations` even though the write path is the new file.
fn compute_database_stats() -> HeliosdoctorDbStats {
    use diesel::Connection;
    use diesel::sqlite::SqliteConnection;

    let write_path = database_write_path();
    let legacy_path = database_legacy_read_path();

    let mut stats = HeliosdoctorDbStats {
        write_db_path: Some(write_path.to_string_lossy().to_string()),
        legacy_db_path: legacy_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        ..Default::default()
    };

    // If the write DB is the same path as legacy (i.e. the write path fell back
    // to .forge.db because no .forge.writes.db exists), only one DB is in play
    // and the table counts come from it directly.
    let only_legacy = legacy_path
        .as_ref()
        .map(|l| l == &write_path)
        .unwrap_or(false);

    let mut conn = match SqliteConnection::establish(write_path.to_string_lossy().as_ref()) {
        Ok(c) => c,
        Err(err) => {
            stats.error = Some(format!("open write db: {err}"));
            return stats;
        }
    };

    if !only_legacy
        && let Some(legacy) = legacy_path.as_ref()
        && legacy.exists()
        && legacy != &write_path
    {
        // ATTACH the legacy DB read-only. The bundled SQLite does not
        // accept a READ ONLY keyword here (both `READ ONLY` and
        // `(READONLY)` fail), so use a plain ATTACH like the pool's
        // SqliteCustomizer fallback. Read-only is enforced
        // structurally: no code path ever writes to `legacy_read`.
        let escaped = legacy.to_string_lossy().replace('\'', "''");
        let attach_sql = format!("ATTACH DATABASE '{}' AS legacy_read", escaped);
        if let Err(err) =
            diesel::connection::SimpleConnection::batch_execute(&mut conn, &attach_sql)
        {
            stats.legacy_attached = Some(false);
            stats.error = Some(format!("attach legacy: {err}"));
        } else {
            stats.legacy_attached = Some(true);
        }
    }

    // Count rows from write DB + legacy DB (when attached) for the four tables
    // most often affected by legacy compression/roundtrip damage.
    for (label, write_table, legacy_table) in [
        (
            "conversations",
            "conversations",
            "legacy_read.conversations",
        ),
        ("messages", "messages", "legacy_read.messages"),
        (
            "context",
            "context_compressions",
            "legacy_read.context_compressions",
        ),
        ("checkpoints", "checkpoints", "legacy_read.checkpoints"),
    ] {
        let legacy_count = if stats.legacy_attached == Some(true) {
            count_table(&mut conn, legacy_table)
        } else {
            None
        };
        let write_count = count_table(&mut conn, write_table);
        stats
            .tables
            .insert(label.to_string(), (write_count, legacy_count));
    }

    // Populate the fields that `heliosdoctor_verbose` reads. In split mode
    // (write DB distinct from legacy) the true total is the union: rows in
    // the write DB plus rows in the legacy DB.
    let write_total = stats
        .tables
        .get("conversations")
        .and_then(|(w, _)| *w)
        .unwrap_or(0)
        .max(0) as u64;
    let legacy_total = stats
        .tables
        .get("conversations")
        .and_then(|(_, l)| *l)
        .unwrap_or(0)
        .max(0) as u64;
    let total = write_total + legacy_total;
    stats.total_conversations = total;

    // Compressed / empty / uncompressed / agent-initiated / oversized rows all
    // come from the `conversations` table (of whichever file holds the bulk —
    // legacy when attached, else the primary). The `context_compressions` /
    // `messages` / `checkpoints` counts above are informational only; the fork
    // schema carries the compression state directly on the row
    // (`is_compressed`, `context_zstd`), so count against those columns.
    // In split mode the per-category counts sum the write DB and the legacy
    // DB so the report matches what `conversations_all` exposes to reads.
    let count_both = |conn: &mut diesel::sqlite::SqliteConnection, predicate: &str| -> u64 {
        let write = count_where(conn, "conversations", predicate)
            .unwrap_or(0)
            .max(0) as u64;
        let legacy = if stats.legacy_attached == Some(true) {
            count_where(conn, "legacy_read.conversations", predicate)
                .unwrap_or(0)
                .max(0) as u64
        } else {
            0
        };
        write + legacy
    };
    stats.compressed_rows = count_both(&mut conn, "is_compressed = 1");
    stats.empty_rows = count_both(&mut conn, "context IS NULL AND context_zstd IS NULL");
    stats.uncompressed_rows = count_both(
        &mut conn,
        "is_compressed IS NOT 1 AND (context IS NOT NULL OR context_zstd IS NOT NULL)",
    );
    stats.agent_initiated_rows = count_both(
        &mut conn,
        "COALESCE(json_extract(context, '$.initiator'), 'user') = 'agent'",
    );
    stats.oversized_rows = count_both(
        &mut conn,
        "length(context_zstd) > 1048576 OR length(context) > 1048576",
    );

    // Real integrity check: primary DB always; legacy DB too when attached.
    use diesel::RunQueryDsl;
    use diesel::sql_types::Text;
    #[derive(diesel::QueryableByName)]
    struct IntegrityRow {
        #[diesel(sql_type = Text)]
        integrity_check: String,
    }
    let check = |conn: &mut diesel::sqlite::SqliteConnection| -> String {
        diesel::sql_query("PRAGMA integrity_check")
            .get_result::<IntegrityRow>(conn)
            .map(|r| r.integrity_check)
            .unwrap_or_else(|_| "unknown".to_string())
    };
    let mut checks: Vec<(String, String)> = vec![("primary".to_string(), check(&mut conn))];
    if stats.legacy_attached == Some(true)
        && let Some(legacy) = legacy_path.as_ref()
        && let Ok(mut legacy_conn) = SqliteConnection::establish(legacy.to_string_lossy().as_ref())
    {
        checks.push(("legacy".to_string(), check(&mut legacy_conn)));
    }
    stats.integrity_check = if checks.iter().all(|(_, c)| c == "ok") {
        "ok".to_string()
    } else {
        checks
            .iter()
            .map(|(name, c)| format!("{name}: {c}"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    stats
}

/// Counts rows in `table` on `conn`. Returns `None` if the table doesn't
/// exist or the query fails.
///
/// `table` must contain only `A-Za-z0-9_.` to prevent SQL injection.
fn count_table(conn: &mut diesel::sqlite::SqliteConnection, table: &str) -> Option<i64> {
    use diesel::sql_types::BigInt;
    use diesel::{QueryableByName, RunQueryDsl};

    if !table
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return None;
    }

    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }

    let sql = format!("SELECT COUNT(*) AS n FROM {}", table);
    diesel::sql_query(sql)
        .get_result::<Row>(conn)
        .ok()
        .map(|r| r.n)
}

/// Counts rows in `table` on `conn` where `predicate` holds. Returns `None`
/// if the query fails.
///
/// `table` and `predicate` must contain only `A-Za-z0-9_. ' =` (plus the
/// literal predicate text) to keep this to a controlled whitelist.
fn count_where(
    conn: &mut diesel::sqlite::SqliteConnection,
    table: &str,
    predicate: &str,
) -> Option<i64> {
    use diesel::sql_types::BigInt;
    use diesel::{QueryableByName, RunQueryDsl};

    // Whitelist covers the fixed predicate literals used by compute_database_stats
    // (is_compressed/context_zstd comparisons, json_extract initiator checks,
    // length() size thresholds). Predicates are compile-time constants, not user
    // input, so this stays injection-safe while permitting the needed operators.
    let allowed = |s: &str| {
        s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || c == '_'
                || c == '.'
                || c == ' '
                || c == '='
                || c == '\''
                || c == '('
                || c == ')'
                || c == '>'
                || c == '<'
                || c == ','
                || c == '$'
                || c == '!'
                || c == '-'
        })
    };
    if !allowed(table) || !allowed(predicate) {
        return None;
    }

    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }

    let sql = format!("SELECT COUNT(*) AS n FROM {} WHERE {}", table, predicate);
    diesel::sql_query(sql)
        .get_result::<Row>(conn)
        .ok()
        .map(|r| r.n)
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
