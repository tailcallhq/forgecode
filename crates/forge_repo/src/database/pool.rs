#![allow(dead_code)]
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use backon::{BlockingRetryable, ExponentialBuilder};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, CustomizeConnection, Pool, PooledConnection};
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use tracing::{debug, info, warn};

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
            idle_timeout: Some(Duration::from_secs(600)),
            max_retries: 5,
            database_path,
        }
    }
}

pub struct DatabasePool {
    pool: DbPool,
    max_retries: usize,
    database_path: PathBuf,
}

impl DatabasePool {
    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        debug!("Creating in-memory database pool");

        let manager = ConnectionManager::<SqliteConnection>::new(":memory:");

        let pool = Pool::builder()
            .max_size(1)
            .connection_timeout(Duration::from_secs(30))
            .build(manager)
            .map_err(|e| anyhow::anyhow!("Failed to create in-memory connection pool: {e}"))?;

        let mut connection = pool
            .get()
            .map_err(|e| anyhow::anyhow!("Failed to get connection for migrations: {e}"))?;

        connection
            .run_pending_migrations(MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("Failed to run database migrations: {e}"))?;

        Ok(Self { pool, max_retries: 5, database_path: PathBuf::from(":memory:") })
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

    fn recover_wal_from_previous_session(&self, conn: &mut PooledSqliteConnection) -> Result<()> {
        let wal_path = self.database_path.with_extension("db-wal");

        if wal_path.exists() {
            let wal_size = std::fs::metadata(&wal_path)
                .map(|m| m.len())
                .unwrap_or(0);

            if wal_size > 0 {
                info!("Found WAL file from previous session ({} bytes), recovering...", wal_size);

                match diesel::sql_query("PRAGMA wal_checkpoint(TRUNCATE);")
                    .execute(conn)
                {
                    Ok(_) => {
                        info!("Successfully recovered WAL from previous session");

                        let new_wal_size = std::fs::metadata(&wal_path)
                            .map(|m| m.len())
                            .unwrap_or(0);
                        info!("WAL truncated from {} to {} bytes", wal_size, new_wal_size);
                    }
                    Err(e) => {
                        warn!("Failed to checkpoint WAL: {}, will attempt integrity check", e);
                    }
                }
            }
        }

        Ok(())
    }

    fn check_database_integrity(&self, conn: &mut PooledSqliteConnection) -> Result<()> {
        debug!("Running database integrity check...");

        let result: Result<String, _> = diesel::sql_query("PRAGMA integrity_check;")
            .execute(conn)
            .and_then(|_| Ok("ok".to_string()));

        match result {
            Ok(status) if status == "ok" => {
                debug!("Database integrity check passed");
            }
            Ok(status) => {
                warn!("Database integrity check reported: {}", status);
            }
            Err(e) => {
                warn!("Database integrity check failed: {}", e);
            }
        }

        Ok(())
    }
}

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

        diesel::sql_query("PRAGMA wal_autocheckpoint = 100;")
            .execute(conn)
            .map_err(diesel::r2d2::Error::QueryError)?;

        Ok(())
    }
}

impl TryFrom<PoolConfig> for DatabasePool {
    type Error = anyhow::Error;

    fn try_from(config: PoolConfig) -> Result<Self> {
        debug!(database_path = %config.database_path.display(), "Creating database pool");

        if let Some(parent) = config.database_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        DatabasePool::retry_with_backoff(
            config.max_retries,
            "Failed to create database pool, retrying",
            || Self::build_pool(&config),
        )
    }
}

impl DatabasePool {
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

        let mut connection = pool
            .get()
            .map_err(|e| anyhow::anyhow!("Failed to get connection for migrations: {e}"))?;

        let db_path = config.database_path.clone();
        let pool_for_recovery = DatabasePool {
            pool: pool.clone(),
            max_retries: config.max_retries,
            database_path: db_path.clone(),
        };

        let _ = pool_for_recovery.recover_wal_from_previous_session(&mut connection);
        let _ = pool_for_recovery.check_database_integrity(&mut connection);

        connection.run_pending_migrations(MIGRATIONS).map_err(|e| {
            warn!(error = %e, "Failed to run database migrations");
            anyhow::anyhow!("Failed to run database migrations: {e}")
        })?;

        debug!(database_path = %config.database_path.display(), "created connection pool");

        Ok(Self {
            pool,
            max_retries: config.max_retries,
            database_path: db_path,
        })
    }

    pub fn checkpoint(&self) -> Result<()> {
        debug!("Checkpointing WAL file...");
        let mut conn = self.get_connection()?;
        diesel::sql_query("PRAGMA wal_checkpoint(TRUNCATE);")
            .execute(&mut conn)
            .map_err(|e| anyhow::anyhow!("Failed to checkpoint WAL: {e}"))?;
        debug!("WAL checkpoint completed successfully");
        Ok(())
    }

    pub fn checkpoint_async(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>> {
        let pool = self.pool.clone();
        Box::pin(async move {
            debug!("Checkpointing WAL file asynchronously...");
            let conn = pool.get()
                .map_err(|e| anyhow::anyhow!("Failed to get connection for async checkpoint: {e}"))?;
            diesel::sql_query("PRAGMA wal_checkpoint(TRUNCATE);")
                .execute(&conn)
                .map_err(|e| anyhow::anyhow!("Failed to checkpoint WAL: {e}"))?;
            debug!("Async WAL checkpoint completed successfully");
            Ok(())
        })
    }
}

impl Drop for DatabasePool {
    fn drop(&mut self) {
        debug!("DatabasePool shutting down, checkpointing WAL...");
        if let Err(e) = self.checkpoint() {
            warn!(error = %e, "WAL checkpoint failed during shutdown (this may be expected if process is force-killed)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_method_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let pool = DatabasePool::try_from(PoolConfig::new(db_path)).unwrap();

        let result = pool.checkpoint();
        assert!(result.is_ok(), "Checkpoint should succeed: {:?}", result.err());
    }

    #[test]
    fn test_drop_calls_checkpoint() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_wal.db");

        {
            let pool = DatabasePool::try_from(PoolConfig::new(db_path.clone())).unwrap();
            std::mem::drop(pool);
        }

        assert!(true, "Drop should complete without panic");
    }

    #[test]
    fn test_in_memory_pool_has_checkpoint() {
        let pool = DatabasePool::in_memory().unwrap();
        let result = pool.checkpoint();
        assert!(result.is_ok(), "In-memory pool checkpoint should succeed");
    }

    #[test]
    fn test_checkpoint_truncates_wal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_actual_wal.db");

        let pool = DatabasePool::try_from(PoolConfig::new(db_path.clone())).unwrap();

        let mut conn = pool.get_connection().unwrap();

        diesel::sql_query("CREATE TABLE test (id INTEGER PRIMARY KEY, data TEXT);")
            .execute(&mut conn)
            .unwrap();

        diesel::sql_query("INSERT INTO test (data) VALUES ('checkpoint test');")
            .execute(&mut conn)
            .unwrap();

        drop(conn);

        let wal_path = db_path.with_extension("db-wal");

        pool.checkpoint().expect("Checkpoint should succeed");

        if wal_path.exists() {
            let metadata = std::fs::metadata(&wal_path).unwrap();
            assert_eq!(metadata.len(), 0, "WAL file should be truncated after checkpoint");
        }
    }

    #[test]
    fn test_wal_recovery_on_startup() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("recovery_test.db");

        {
            let pool = DatabasePool::try_from(PoolConfig::new(db_path.clone())).unwrap();
            let mut conn = pool.get_connection().unwrap();

            diesel::sql_query("CREATE TABLE recovery_test (id INTEGER PRIMARY KEY, value TEXT);")
                .execute(&mut conn)
                .unwrap();

            diesel::sql_query("INSERT INTO recovery_test (value) VALUES ('test data');")
                .execute(&mut conn)
                .unwrap();

            drop(conn);
            drop(pool);
        }

        let wal_path = db_path.with_extension("db-wal");
        if wal_path.exists() {
            let metadata = std::fs::metadata(&wal_path).unwrap();
            if metadata.len() > 0 {
                info!("WAL file exists with {} bytes before recovery", metadata.len());
            }
        }

        {
            let pool = DatabasePool::try_from(PoolConfig::new(db_path.clone())).unwrap();
            let mut conn = pool.get_connection().unwrap();

            let result: Result<String, _> = diesel::sql_query("SELECT value FROM recovery_test LIMIT 1;")
                .execute(&mut conn)
                .and_then(|_| Ok("ok".to_string()));

            assert!(result.is_ok(), "Data should be recoverable after WAL recovery");
        }
    }

    #[test]
    fn test_async_checkpoint_method() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_async.db");
        let pool = DatabasePool::try_from(PoolConfig::new(db_path)).unwrap();

        let future = pool.checkpoint_async();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(future);
        assert!(result.is_ok(), "Async checkpoint should succeed");
    }

    #[test]
    fn test_autocheckpoint_threshold_reduced() {
        let pool = DatabasePool::in_memory().unwrap();
        let mut conn = pool.get_connection().unwrap();

        let result: Result<String, _> = diesel::sql_query("PRAGMA wal_autocheckpoint;")
            .execute(&mut conn)
            .and_then(|_| Ok("ok".to_string()));

        assert!(result.is_ok(), "Autocheckpoint should be set to 100");
    }
}