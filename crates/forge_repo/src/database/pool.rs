#![allow(dead_code)]
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use backon::{BlockingRetryable, ExponentialBuilder};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, CustomizeConnection, Pool, PooledConnection};
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use tracing::{debug, warn};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("src/database/migrations");

pub type DbPool = Pool<ConnectionManager<SqliteConnection>>;
pub type PooledSqliteConnection = PooledConnection<ConnectionManager<SqliteConnection>>;

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub max_size: u32,
    pub min_idle: Option<u32>,
    pub connection_timeout: Duration,
    pub idle_timeout: Option<Duration>,
    pub max_retries: usize,
    pub database_path: PathBuf,
}

impl PoolConfig {
    pub fn new(database_path: PathBuf) -> Self {
        Self {
            max_size: 5,
            min_idle: Some(1),
            connection_timeout: Duration::from_secs(5),
            idle_timeout: Some(Duration::from_secs(600)), // 10 minutes
            max_retries: 5,
            database_path,
        }
    }
}

pub struct DatabasePool {
    /// Lazily-initialized connection pool. Building the pool eagerly at
    /// startup can fail (e.g. the SQLite file is locked by another process),
    /// which previously crashed the app. By deferring initialization to the
    /// first connection request, failures surface as recoverable errors from
    /// repository methods instead of a startup panic.
    pool: Mutex<Option<DbPool>>,
    config: PoolConfig,
}

impl DatabasePool {
    /// Creates a database pool handle without opening any connections.
    ///
    /// The underlying pool is created lazily on the first call to
    /// `get_connection`, so this constructor is infallible.
    pub fn new(config: PoolConfig) -> Self {
        Self { pool: Mutex::new(None), config }
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        debug!("Creating in-memory database pool");

        let manager = ConnectionManager::<SqliteConnection>::new(":memory:");

        let pool = Pool::builder()
            .max_size(1) // Single connection for in-memory testing
            .connection_timeout(Duration::from_secs(30))
            .build(manager)
            .map_err(|e| anyhow::anyhow!("Failed to create in-memory connection pool: {e}"))?;

        // Run migrations on the in-memory database
        let mut connection = pool
            .get()
            .map_err(|e| anyhow::anyhow!("Failed to get connection for migrations: {e}"))?;

        connection
            .run_pending_migrations(MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("Failed to run database migrations: {e}"))?;

        Ok(Self {
            pool: Mutex::new(Some(pool)),
            config: PoolConfig::new(PathBuf::from(":memory:")),
        })
    }

    /// Returns the underlying pool, building it on first use.
    ///
    /// # Errors
    /// Returns an error if the pool cannot be created after retrying, for
    /// example when the SQLite database file is locked by another process or
    /// is not readable.
    fn pool(&self) -> Result<DbPool> {
        let mut guard = self
            .pool
            .lock()
            .map_err(|_| anyhow::anyhow!("Database pool mutex poisoned"))?;

        if let Some(pool) = guard.as_ref() {
            return Ok(pool.clone());
        }

        // Retry pool creation with exponential backoff to handle transient
        // failures such as another process holding an exclusive lock on the
        // SQLite database file.
        let pool = Self::retry_with_backoff(
            self.config.max_retries,
            "Failed to create database pool, retrying",
            || Self::build_pool(&self.config),
        )?;

        *guard = Some(pool.clone());
        Ok(pool)
    }

    /// Retrieves a connection from the pool, initializing the pool on first
    /// use.
    ///
    /// # Errors
    /// Returns an error if the pool cannot be created or a connection cannot
    /// be acquired after retrying.
    pub fn get_connection(&self) -> Result<PooledSqliteConnection> {
        let pool = self.pool()?;
        Self::retry_with_backoff(
            self.config.max_retries,
            "Failed to get connection from pool, retrying",
            || {
                pool.get()
                    .map_err(|e| anyhow::anyhow!("Failed to get connection from pool: {e}"))
            },
        )
    }

    /// Retries a blocking database pool operation with exponential backoff.
    fn retry_with_backoff<T>(
        max_retries: usize,
        message: &'static str,
        operation: impl FnMut() -> Result<T>,
    ) -> Result<T> {
        operation
            .retry(
                ExponentialBuilder::default()
                    .with_min_delay(Duration::from_secs(1))
                    .with_max_times(max_retries)
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
// Configure SQLite for better concurrency ref: https://docs.diesel.rs/master/diesel/sqlite/struct.SqliteConnection.html#concurrency
#[derive(Debug)]
struct SqliteCustomizer;

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
        diesel::sql_query("PRAGMA wal_autocheckpoint = 1000;")
            .execute(conn)
            .map_err(diesel::r2d2::Error::QueryError)?;
        Ok(())
    }
}

impl DatabasePool {
    /// Builds the connection pool and runs migrations.
    ///
    /// # Errors
    /// Returns an error if the database directory cannot be created, the pool
    /// cannot be built, or migrations fail.
    fn build_pool(config: &PoolConfig) -> Result<DbPool> {
        debug!(database_path = %config.database_path.display(), "Creating database pool");

        // Ensure the parent directory exists
        if let Some(parent) = config.database_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let database_url = config.database_path.to_string_lossy().to_string();
        let manager = ConnectionManager::<SqliteConnection>::new(&database_url);

        let mut builder = Pool::builder()
            .max_size(config.max_size)
            .connection_timeout(config.connection_timeout)
            .connection_customizer(Box::new(SqliteCustomizer));

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

        debug!(database_path = %config.database_path.display(), "created connection pool");
        Ok(pool)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// A pool config pointing at an unopenable database path with fast
    /// retries so failure tests complete quickly.
    fn broken_config() -> PoolConfig {
        let mut config = PoolConfig::new(PathBuf::from("/dev/null/forge.db"));
        config.max_retries = 0;
        config.connection_timeout = Duration::from_millis(100);
        config
    }

    #[test]
    fn test_new_with_broken_path_does_not_panic_or_connect() {
        // Regression test: constructing the pool must never fail or panic,
        // even when the database path is unusable. Previously this crashed
        // the app at startup via `TryFrom` + `unwrap()`.
        let fixture = broken_config();

        let actual = DatabasePool::new(fixture);

        // No pool should have been created yet (lazy initialization).
        let expected = true;
        assert_eq!(actual.pool.lock().unwrap().is_none(), expected);
    }

    #[test]
    fn test_get_connection_with_broken_path_returns_error() {
        // Regression test: a broken database must surface as a recoverable
        // error from `get_connection`, not a panic.
        let fixture = DatabasePool::new(broken_config());

        let actual = fixture.get_connection();

        assert!(actual.is_err());
    }

    #[test]
    fn test_get_connection_initializes_pool_lazily() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let fixture = DatabasePool::new(PoolConfig::new(dir.path().join("forge.db")));

        // Pool must not exist before the first connection request.
        assert!(fixture.pool.lock().unwrap().is_none());

        let actual = fixture.get_connection();

        assert!(actual.is_ok());
        // Pool must be cached after the first successful connection.
        let expected = true;
        assert_eq!(fixture.pool.lock().unwrap().is_some(), expected);
        Ok(())
    }
}
