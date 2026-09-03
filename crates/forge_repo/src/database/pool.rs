#![allow(dead_code)]
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use backon::{BlockingRetryable, ExponentialBuilder};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, CustomizeConnection, Pool, PooledConnection};
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use forge_config::RetryConfig;
use tracing::{debug, warn};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("src/database/migrations");

pub type DbPool = Pool<ConnectionManager<SqliteConnection>>;
pub type PooledSqliteConnection = PooledConnection<ConnectionManager<SqliteConnection>>;

/// Fallback max retries for pool operations when no `RetryConfig` is supplied.
const DEFAULT_POOL_MAX_RETRIES: usize = 5;
/// Fallback minimum delay between pool-connection retries.
const DEFAULT_POOL_MIN_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub max_size: u32,
    pub min_idle: Option<u32>,
    pub connection_timeout: Duration,
    pub idle_timeout: Option<Duration>,
    pub database_path: PathBuf,
    /// Optional path to a *legacy* read-only database that should be
    /// ATTACHed on every connection acquire and unioned into the local
    /// `conversations` table via the `conversations_all` TEMP VIEW.
    ///
    /// When `None`, or when the legacy path equals `database_path`, or the
    /// legacy file is missing, the read-side UNION collapses to the local
    /// table only.
    pub legacy_database_path: Option<PathBuf>,
    /// Retry/backoff configuration for transient pool-creation and
    /// connection-acquisition failures.  When `None` the pool falls back to
    /// hard-coded defaults (`DEFAULT_POOL_MAX_RETRIES`,
    /// `DEFAULT_POOL_MIN_DELAY`).
    pub retry_config: Option<RetryConfig>,
}

impl PoolConfig {
    pub fn new(database_path: PathBuf) -> Self {
        Self {
            max_size: 5,
            min_idle: Some(1),
            connection_timeout: Duration::from_secs(5),
            idle_timeout: Some(Duration::from_secs(600)), // 10 minutes
            database_path,
            legacy_database_path: None,
            retry_config: None,
        }
    }

    /// Attach a [`RetryConfig`] so pool-level retries honour the unified
    /// system-wide settings rather than the hard-coded defaults.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = Some(config);
        self
    }

    /// Attach a legacy read-only database path for the split-DB read UNION.
    pub fn with_legacy_database_path(mut self, legacy: Option<PathBuf>) -> Self {
        self.legacy_database_path = legacy;
        self
    }
}
pub struct DatabasePool {
    pool: DbPool,
    retry_config: RetryConfig,
    database_path: PathBuf,
    legacy_database_path: Option<PathBuf>,
    _checkpointer: Option<crate::database::checkpoint::WalCheckpointer>,
}

impl DatabasePool {
    /// Returns the resolved SQLite database path this pool was built for.
    /// Used by `migrate_data_dir` to discover the legacy directory.
    pub fn database_path(&self) -> &std::path::Path {
        &self.database_path
    }

    /// Returns the resolved legacy database path, if one was attached.
    pub fn legacy_database_path(&self) -> Option<&std::path::Path> {
        self.legacy_database_path.as_deref()
    }
    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        debug!("Creating in-memory database pool");

        let manager = ConnectionManager::<SqliteConnection>::new(":memory:");

        let customizer = SqliteCustomizer {
            primary_database_path: PathBuf::from(":memory:"),
            legacy_database_path: None,
        };

        let pool = Pool::builder()
            .max_size(1) // Single connection for in-memory testing
            .connection_timeout(Duration::from_secs(30))
            .connection_customizer(Box::new(customizer.clone()))
            .build(manager)
            .map_err(|e| anyhow::anyhow!("Failed to create in-memory connection pool: {e}"))?;

        // Run migrations on the in-memory database
        let mut connection = pool
            .get()
            .map_err(|e| anyhow::anyhow!("Failed to get connection for migrations: {e}"))?;

        connection
            .run_pending_migrations(MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("Failed to run database migrations: {e}"))?;

        // `on_acquire` ran before migrations on this fresh in-memory DB, so
        // create the read view now that the schema exists.
        customizer.configure_read_projection(&mut connection);

        Ok(Self {
            pool,
            retry_config: RetryConfig::default(),
            database_path: PathBuf::from(":memory:"),
            legacy_database_path: None,
            _checkpointer: None,
        })
    }

    pub fn get_connection(&self) -> Result<PooledSqliteConnection> {
        Self::retry_with_backoff(
            &self.retry_config,
            "Failed to get connection from pool, retrying",
            || {
                self.pool
                    .get()
                    .map_err(|e| anyhow::anyhow!("Failed to get connection from pool: {e}"))
            },
        )
    }

    /// Retries a blocking database pool operation with exponential backoff
    /// driven by the provided [`RetryConfig`].
    ///
    /// `RetryConfig` fields map to the backoff strategy as follows:
    /// - `max_attempts`      → `with_max_times`
    /// - `min_delay_ms`      → `with_min_delay` (falls back to
    ///   [`DEFAULT_POOL_MIN_DELAY`] when zero)
    /// - `backoff_factor`    → `with_factor` (falls back to `2.0` when zero)
    pub(crate) fn retry_with_backoff<T>(
        retry_config: &RetryConfig,
        message: &'static str,
        operation: impl FnMut() -> Result<T>,
    ) -> Result<T> {
        let max_times = if retry_config.max_attempts > 0 {
            retry_config.max_attempts
        } else {
            DEFAULT_POOL_MAX_RETRIES
        };

        let min_delay = if retry_config.min_delay_ms > 0 {
            Duration::from_millis(retry_config.min_delay_ms)
        } else {
            DEFAULT_POOL_MIN_DELAY
        };

        let factor = if retry_config.backoff_factor > 0 {
            retry_config.backoff_factor as f32
        } else {
            2.0_f32
        };

        operation
            .retry(
                ExponentialBuilder::default()
                    .with_min_delay(min_delay)
                    .with_max_times(max_times)
                    .with_factor(factor)
                    .with_jitter(),
            )
            .sleep(std::thread::sleep)
            .notify(|err, dur| {
                warn!(
                    error = %err,
                    retry_after_ms = dur.as_millis() as u64,
                    "{}",
                    message
                );
            })
            .call()
    }
}
/// Configure SQLite for better concurrency and storage efficiency.
///
/// Ref: https://docs.diesel.rs/master/diesel/sqlite/struct.SqliteConnection.html#concurrency
///
/// **auto_vacuum=INCREMENTAL:**
/// - For NEW databases: enables incremental auto_vacuum at creation time,
///   allowing freed pages to return to the OS continuously without an
///   exclusive-lock full VACUUM.
/// - For EXISTING databases: this pragma is a no-op and doesn't change the
///   setting. To convert an existing database to INCREMENTAL auto_vacuum, run a
///   one-time full `VACUUM` (e.g., via forge-vacuum tool). After that one-time
///   conversion, the background checkpointer's incremental_vacuum keeps
///   reclaiming freed pages automatically.
///
/// **FORGE_INCREMENTAL_VACUUM env var (default: enabled):**
/// - When enabled, the background checkpoint task periodically runs `PRAGMA
///   incremental_vacuum` after truncating the WAL, to return freed pages (from
///   P4 prune, zstd compression, deletes) to the OS.
/// - Set to "0" or "false" to disable if needed.
#[derive(Debug, Clone)]
struct SqliteCustomizer {
    /// Primary (write) database path, used to guard against ATTACHing the
    /// legacy DB when it resolves to the same file.
    primary_database_path: PathBuf,
    /// Optional legacy DB to ATTACH read-only and expose via the
    /// `conversations_all` TEMP VIEW. When `None` (or pointing at the
    /// same path, or the file does not exist) the read-side UNION
    /// collapses to the local `conversations` table.
    legacy_database_path: Option<PathBuf>,
}

impl SqliteCustomizer {
    /// ATTACHes the legacy DB (when present and distinct from the primary)
    /// and creates the `conversations_all` TEMP VIEW that the read layer
    /// queries. The view is **always** created — a plain
    /// `SELECT * FROM conversations` when no legacy DB is attached — so
    /// the query layer has a single stable read target regardless of
    /// configuration.
    ///
    /// On connections acquired before migrations run (fresh databases) the
    /// `conversations` table does not exist yet and both CREATEs fail; the
    /// migration path in [`DatabasePool`] calls this again after running
    /// migrations on the migration connection.
    fn configure_read_projection(&self, conn: &mut SqliteConnection) {
        // ATTACH the legacy DB read-only when it exists and is a distinct
        // file from the primary. Errors are tolerated: if the ATTACH fails,
        // the UNION view creation below fails and we fall back to a plain
        // view over the local table.
        let mut legacy_attached = false;
        if let Some(legacy_path) = &self.legacy_database_path {
            let same_file = self
                .primary_database_path
                .canonicalize()
                .ok()
                .zip(legacy_path.canonicalize().ok())
                .map(|(primary, legacy)| primary == legacy)
                .unwrap_or(false);
            if !same_file && legacy_path.exists() {
                let canonical_legacy = legacy_path
                    .canonicalize()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| legacy_path.display().to_string());
                let escaped = canonical_legacy.replace('\'', "''");
                // `READ ONLY` requires SQLite >= 3.37; this binary links the
                // bundled libsqlite3-sys (SQLite 3.51.3). Fall back to a
                // plain ATTACH for older runtimes: read-only enforcement is
                // then structural, since no code path writes to `legacy_read`.
                let attach_ro = format!("ATTACH DATABASE '{escaped}' AS legacy_read READ ONLY");
                let attach_plain = format!("ATTACH DATABASE '{escaped}' AS legacy_read");
                let mut attach_ok = diesel::sql_query(&attach_ro).execute(conn).is_ok();
                if !attach_ok {
                    attach_ok = diesel::sql_query(&attach_plain).execute(conn).is_ok();
                }
                if attach_ok {
                    legacy_attached = true;
                }
            }
        }

        if legacy_attached {
            // Union of local + legacy rows. SQLite resolves view bodies
            // lazily, so this succeeds even on a fresh primary where
            // `conversations` does not exist yet; the view becomes usable
            // once migrations have created the table. If the union cannot be
            // created (e.g. `legacy_read` was not attached), fall through to
            // the plain view.
            if diesel::sql_query(
                "CREATE TEMP VIEW IF NOT EXISTS conversations_all AS \
                 SELECT * FROM conversations \
                 UNION ALL \
                 SELECT * FROM legacy_read.conversations",
            )
            .execute(conn)
            .is_ok()
            {
                return;
            }
        }

        let _ = diesel::sql_query(
            "CREATE TEMP VIEW IF NOT EXISTS conversations_all AS \
             SELECT * FROM conversations",
        )
        .execute(conn);
    }
}

impl CustomizeConnection<SqliteConnection, diesel::r2d2::Error> for SqliteCustomizer {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), diesel::r2d2::Error> {
        diesel::sql_query("PRAGMA busy_timeout = 30000;")
            .execute(conn)
            .map_err(diesel::r2d2::Error::QueryError)?;
        diesel::sql_query("PRAGMA journal_mode = WAL;")
            .execute(conn)
            .map_err(diesel::r2d2::Error::QueryError)?;
        diesel::sql_query("PRAGMA synchronous = NORMAL;")
            .execute(conn)
            .map_err(diesel::r2d2::Error::QueryError)?;
        // Phenotype-org change: many forge processes share one .forge.db.
        // Per-connection PASSIVE autocheckpoint mostly no-ops under contention
        // while still costing writers, so disable it here and move checkpointing
        // to a dedicated background thread (see checkpoint.rs).
        diesel::sql_query("PRAGMA wal_autocheckpoint = 0;")
            .execute(conn)
            .map_err(diesel::r2d2::Error::QueryError)?;
        // Enable incremental auto_vacuum for new databases. On existing DBs, this is a
        // no-op; they need one full VACUUM to convert, after which
        // incremental_vacuum (spawned in the background checkpointer) keeps
        // reclaiming pages automatically.
        diesel::sql_query("PRAGMA auto_vacuum = INCREMENTAL;")
            .execute(conn)
            .map_err(diesel::r2d2::Error::QueryError)?;

        // Split-DB read UNION: ATTACH the legacy DB read-only and expose
        // its `conversations` table as `legacy_read.conversations`. The
        // TEMP VIEW `conversations_all` is the read-side projection that
        // SELECT queries should target; writes still go to `conversations`
        // on the primary database.
        self.configure_read_projection(conn);

        Ok(())
    }
}

impl TryFrom<PoolConfig> for DatabasePool {
    type Error = anyhow::Error;

    fn try_from(config: PoolConfig) -> Result<Self> {
        debug!(database_path = %config.database_path.display(), "Creating database pool");

        // Ensure the parent directory exists
        if let Some(parent) = config.database_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Retry pool creation with exponential backoff to handle transient
        // failures such as another process holding an exclusive lock on the
        // SQLite database file.
        let retry_config = config.retry_config.clone().unwrap_or_default();
        DatabasePool::retry_with_backoff(
            &retry_config,
            "Failed to create database pool, retrying",
            || Self::build_pool(&config, retry_config.clone()),
        )
    }
}

impl DatabasePool {
    /// Builds the connection pool and runs migrations.
    fn build_pool(config: &PoolConfig, retry_config: RetryConfig) -> Result<Self> {
        let database_url = config.database_path.to_string_lossy().to_string();
        let manager = ConnectionManager::<SqliteConnection>::new(&database_url);

        let customizer = SqliteCustomizer {
            primary_database_path: config.database_path.clone(),
            legacy_database_path: config.legacy_database_path.clone(),
        };

        let mut builder = Pool::builder()
            .max_size(config.max_size)
            .connection_timeout(config.connection_timeout)
            .connection_customizer(Box::new(customizer.clone()));

        if let Some(min_idle) = config.min_idle {
            builder = builder.min_idle(Some(min_idle));
        }

        if let Some(idle_timeout) = config.idle_timeout {
            builder = builder.idle_timeout(Some(idle_timeout));
        }

        let pool = builder.build(manager).map_err(|e| {
            warn!(error = %e, "Failed to create connection pool");
            anyhow::anyhow!("Failed to create connection pool: {e}")
        })?;

        // Run migrations on a connection from the pool
        let mut connection = pool
            .get()
            .map_err(|e| anyhow::anyhow!("Failed to get connection for migrations: {e}"))?;

        connection.run_pending_migrations(MIGRATIONS).map_err(|e| {
            warn!(error = %e, "Failed to run database migrations");
            anyhow::anyhow!("Failed to run database migrations: {e}")
        })?;

        // `on_acquire` runs before migrations on a fresh database, so the
        // `conversations_all` view could not be created yet on the migration
        // connection. Create it now that the schema exists.
        customizer.configure_read_projection(&mut connection);

        let checkpointer =
            crate::database::checkpoint::WalCheckpointer::spawn(config.database_path.clone());

        debug!(database_path = %config.database_path.display(), "created connection pool");
        Ok(Self {
            pool,
            retry_config,
            database_path: config.database_path.clone(),
            legacy_database_path: config.legacy_database_path.clone(),
            _checkpointer: checkpointer,
        })
    }
}
