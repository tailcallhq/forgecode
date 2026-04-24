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
            min_idle: None,
            connection_timeout: Duration::from_secs(15),
            idle_timeout: Some(Duration::from_secs(600)), // 10 minutes
            max_retries: 5,
            database_path,
        }
    }
}

pub struct DatabasePool {
    pool: Mutex<DbPool>,
    config: PoolConfig,
}

impl DatabasePool {
    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        debug!("Creating in-memory database pool");

        let manager = ConnectionManager::<SqliteConnection>::new(":memory:");

        let pool = Pool::builder()
            .max_size(1) // Single connection for in-memory testing
            .connection_timeout(Duration::from_secs(30))
            .connection_customizer(Box::new(SqliteCustomizer))
            .build(manager)
            .map_err(|e| anyhow::anyhow!("Failed to create in-memory connection pool: {e}"))?;

        // Run migrations on the in-memory database
        let mut connection = pool
            .get()
            .map_err(|e| anyhow::anyhow!("Failed to get connection for migrations: {e}"))?;

        connection
            .run_pending_migrations(MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("Failed to run database migrations: {e}"))?;

        let config = PoolConfig::new(PathBuf::from(":memory:"));
        Ok(Self { pool: Mutex::new(pool), config })
    }

    /// Gets a connection from the pool with retry and self-healing.
    ///
    /// If all retries fail, the pool is recreated from scratch as a last resort
    /// before attempting one final connection checkout.
    pub fn get_connection(&self) -> Result<PooledSqliteConnection> {
        let max_retries = self.config.max_retries;
        let pool = self.pool.lock().expect("DatabasePool mutex poisoned");

        let result = Self::retry_with_backoff(
            max_retries,
            "Failed to get connection from pool, retrying",
            || {
                pool.get()
                    .map_err(|e| anyhow::anyhow!("Failed to get connection from pool: {e}"))
            },
        );

        match result {
            Ok(conn) => Ok(conn),
            Err(original_error) => {
                warn!(
                    error = %original_error,
                    "All retries exhausted, attempting pool recreation as last resort"
                );
                drop(pool);
                self.recreate_pool()?;
                let pool = self.pool.lock().expect("DatabasePool mutex poisoned");
                pool.get()
                    .map_err(|e| anyhow::anyhow!("Failed to get connection after pool recreation: {e}"))
            }
        }
    }

    /// Recreates the connection pool from scratch using the stored configuration.
    ///
    /// This is used as a last-resort recovery mechanism when all retry attempts
    /// have failed, typically due to stale or corrupted connections after long
    /// idle periods.
    fn recreate_pool(&self) -> Result<()> {
        debug!(
            database_path = %self.config.database_path.display(),
            "Recreating database pool from scratch"
        );

        let new_database_pool = Self::retry_with_backoff(
            self.config.max_retries,
            "Failed to recreate database pool, retrying",
            || Self::build_pool(&self.config),
        )?;

        let new_pool = new_database_pool
            .pool
            .into_inner()
            .expect("DatabasePool mutex should not be poisoned during recreation");

        let mut guard = self.pool.lock().expect("DatabasePool mutex poisoned");
        *guard = new_pool;
        Ok(())
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
        // Health check: verify the connection is alive before handing it out.
        // This catches stale connections that were evicted and recreated during
        // idle periods.
        diesel::sql_query("SELECT 1")
            .execute(conn)
            .map_err(diesel::r2d2::Error::QueryError)?;

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
        // Checkpoint the WAL to ensure a clean state, especially important
        // after long idle periods where the WAL may have grown or become
        // inconsistent.
        diesel::sql_query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(conn)
            .map_err(diesel::r2d2::Error::QueryError)?;
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
        DatabasePool::retry_with_backoff(
            config.max_retries,
            "Failed to create database pool, retrying",
            || Self::build_pool(&config),
        )
    }
}

impl DatabasePool {
    /// Builds the connection pool and runs migrations.
    fn build_pool(config: &PoolConfig) -> Result<Self> {
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
        Ok(Self {
            pool: Mutex::new(pool),
            config: config.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn pool_with_short_idle_timeout() -> anyhow::Result<DatabasePool> {
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("test.sqlite");
        let config = PoolConfig {
            max_size: 5,
            min_idle: None,
            connection_timeout: Duration::from_secs(15),
            idle_timeout: Some(Duration::from_millis(100)),
            max_retries: 3,
            database_path: db_path,
        };
        DatabasePool::try_from(config)
    }

    #[test]
    fn test_idle_eviction_recovery() -> anyhow::Result<()> {
        let pool = pool_with_short_idle_timeout()?;

        // Get a connection to warm up the pool
        {
            let mut conn = pool.get_connection()?;
            diesel::sql_query("SELECT 1").execute(&mut *conn)?;
        }

        // Wait for idle timeout to evict connections
        std::thread::sleep(Duration::from_millis(300));

        // After eviction, get_connection should still succeed by creating a fresh
        // connection
        let mut conn = pool.get_connection()?;
        let actual = diesel::sql_query("SELECT 1 AS result")
            .execute(&mut *conn)
            .unwrap();
        let expected = 1;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn test_health_check_on_acquire() -> anyhow::Result<()> {
        let pool = DatabasePool::in_memory()?;

        // Every get_connection should succeed because on_acquire runs SELECT 1
        // to validate the connection
        for _ in 0..5 {
            let mut conn = pool.get_connection()?;
            let actual = diesel::sql_query("SELECT 1 AS result")
                .execute(&mut *conn)
                .unwrap();
            let expected = 1;
            assert_eq!(actual, expected);
        }
        Ok(())
    }

    #[test]
    fn test_pool_config_defaults() {
        let config = PoolConfig::new(PathBuf::from("/tmp/test.sqlite"));

        assert_eq!(config.max_size, 5);
        assert_eq!(config.min_idle, None);
        assert_eq!(config.connection_timeout, Duration::from_secs(15));
        assert_eq!(config.idle_timeout, Some(Duration::from_secs(600)));
        assert_eq!(config.max_retries, 5);
    }

    #[test]
    fn test_pool_recreation_after_simulated_failure() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("test_recreate.sqlite");
        let config = PoolConfig {
            max_size: 2,
            min_idle: None,
            connection_timeout: Duration::from_secs(15),
            idle_timeout: Some(Duration::from_millis(100)),
            max_retries: 3,
            database_path: db_path.clone(),
        };
        let pool = DatabasePool::try_from(config)?;

        // Use the pool normally
        {
            let mut conn = pool.get_connection()?;
            diesel::sql_query("SELECT 1").execute(&mut *conn)?;
        }

        // Wait for idle eviction
        std::thread::sleep(Duration::from_millis(300));

        // Recreate the pool manually to verify it works
        pool.recreate_pool()?;

        // Verify the recreated pool works by running a query
        let mut conn = pool.get_connection()?;
        let result: Result<i32, _> = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>("1")).first(&mut *conn);
        assert!(result.is_ok(), "Pool should be usable after recreation");
        Ok(())
    }

    #[test]
    fn test_wal_checkpoint_on_acquire() -> anyhow::Result<()> {
        let pool = DatabasePool::in_memory()?;

        // The on_acquire hook runs PRAGMA wal_checkpoint(TRUNCATE).
        // For an in-memory DB this is a no-op but should not error.
        let mut conn = pool.get_connection()?;

        // Verify the connection is usable after all PRAGMAs
        let actual = diesel::sql_query("SELECT 1 AS result")
            .execute(&mut *conn)
            .unwrap();
        let expected = 1;
        assert_eq!(actual, expected);
        Ok(())
    }
}
