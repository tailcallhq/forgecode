#![allow(dead_code)]
use std::path::PathBuf;
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
    pool: DbPool,
    max_retries: usize,
}

impl DatabasePool {
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

        Ok(Self { pool, max_retries: 5 })
    }

    pub fn get_connection(&self) -> Result<PooledSqliteConnection> {
        Self::retry_with_backoff(
            self.max_retries,
            "Failed to get connection from pool, retrying",
            || {
                self.pool
                    .get()
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
        Ok(Self { pool, max_retries: config.max_retries })
    }
}
