//! Phenotype-org addition for WAL contention control and incremental vacuum in
//! a shared `.forge.db`.
//!
//! Many forge processes can point at the same SQLite database file.
//! Per-connection passive autocheckpointing tends to no-op under contention
//! because readers or writers often keep frames pinned, but every writer still
//! pays the checkpoint attempt cost. This module dedicates one background
//! thread per process to periodically probe the WAL and truncate it when it is
//! large enough to matter.
//!
//! After each checkpoint, if enabled via `FORGE_INCREMENTAL_VACUUM` (default:
//! enabled), it also runs `PRAGMA incremental_vacuum` to return freed pages
//! (from P4 prune, zstd compression, deletes) to the OS without an
//! exclusive-lock full VACUUM.
//!
//! SQLite serialises checkpoints and vacuums across processes, so only one
//! process will successfully truncate/vacuum at a time while the others observe
//! `busy` and skip. That means we do not need process-wide election or
//! coordination: each process can own one best-effort checkpointer, and the
//! database file will still be reclaimed safely.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use diesel::QueryableByName;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel::sql_types::Integer;
use diesel::sqlite::SqliteConnection;
use tracing::{debug, warn};

#[derive(Debug)]
pub struct WalCheckpointer {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

#[derive(QueryableByName)]
struct CheckpointRow {
    #[diesel(sql_type = Integer)]
    busy: i32,
    #[diesel(sql_type = Integer)]
    log: i32,
    #[diesel(sql_type = Integer, column_name = checkpointed)]
    _checkpointed: i32,
}

impl WalCheckpointer {
    pub fn spawn(database_path: PathBuf) -> Option<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);

        let handle = thread::Builder::new()
            .name("forge-wal-checkpointer".to_owned())
            .spawn(move || run_checkpointer(database_path, thread_stop))
            .map_err(|error| {
                warn!(error = %error, "failed to spawn WAL checkpointer thread");
            })
            .ok()?;

        Some(Self { stop, handle: Some(handle) })
    }
}

impl Drop for WalCheckpointer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);

        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_checkpointer(database_path: PathBuf, stop: Arc<AtomicBool>) {
    let database_url = database_path.to_string_lossy().to_string();
    let mut connection = match SqliteConnection::establish(&database_url) {
        Ok(connection) => connection,
        Err(error) => {
            warn!(error = %error, database_path = %database_path.display(), "failed to open WAL checkpointer connection");
            return;
        }
    };

    if let Err(error) = connection.batch_execute("PRAGMA busy_timeout = 10000;") {
        debug!(error = %error, "failed to configure WAL checkpointer busy timeout");
        return;
    }

    // Read configurable constants from environment, with defaults and clamping.
    let checkpoint_secs = std::env::var("FORGE_WAL_CHECKPOINT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5)
        .max(1);

    let wal_frame_floor = std::env::var("FORGE_WAL_FRAME_FLOOR")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(256)
        .max(0);

    debug!(
        checkpoint_secs = checkpoint_secs,
        wal_frame_floor = wal_frame_floor,
        "WAL checkpointer configuration loaded"
    );

    loop {
        if sleep_with_stop(&stop, Duration::from_secs(checkpoint_secs)) {
            run_final_checkpoint(&mut connection);
            return;
        }

        match wal_checkpoint_passive(&mut connection) {
            Ok(row) if row.log < wal_frame_floor => {
                debug!(
                    log_frames = row.log,
                    threshold = wal_frame_floor,
                    "WAL checkpoint skipped; log below threshold"
                );
            }
            Ok(_) => {
                run_truncate_checkpoint(&mut connection);
            }
            Err(error) => {
                debug!(error = %error, "failed to probe WAL checkpoint state");
            }
        }
    }
}

fn sleep_with_stop(stop: &Arc<AtomicBool>, interval: Duration) -> bool {
    let slice = Duration::from_millis(250);
    let mut elapsed = Duration::ZERO;

    while elapsed < interval {
        if stop.load(Ordering::SeqCst) {
            return true;
        }

        let remaining = interval.saturating_sub(elapsed);
        let step = slice.min(remaining);
        thread::sleep(step);
        elapsed += step;
    }

    stop.load(Ordering::SeqCst)
}

fn wal_checkpoint_passive(connection: &mut SqliteConnection) -> Result<CheckpointRow, DieselError> {
    diesel::sql_query("PRAGMA wal_checkpoint(PASSIVE);").get_result(connection)
}

fn wal_checkpoint_truncate(
    connection: &mut SqliteConnection,
) -> Result<CheckpointRow, DieselError> {
    diesel::sql_query("PRAGMA wal_checkpoint(TRUNCATE);").get_result(connection)
}

fn run_truncate_checkpoint(connection: &mut SqliteConnection) {
    match wal_checkpoint_truncate(connection) {
        Ok(row) if row.busy != 0 => {
            debug!(
                busy = row.busy,
                log_frames = row.log,
                "checkpoint busy; skipping"
            );
        }
        Ok(row) => {
            debug!(
                busy = row.busy,
                log_frames = row.log,
                "checkpoint truncated WAL"
            );

            // If incremental vacuum is enabled, reclaim freed pages after checkpoint.
            // This returns pages (from P4 prune, zstd compression, deletes) to the OS
            // without an exclusive-lock full VACUUM.
            if is_incremental_vacuum_enabled() {
                run_incremental_vacuum(connection);
            }
        }
        Err(error) => {
            debug!(error = %error, "failed to truncate WAL checkpoint");
        }
    }
}

fn run_final_checkpoint(connection: &mut SqliteConnection) {
    match wal_checkpoint_truncate(connection) {
        Ok(row) if row.busy != 0 => {
            debug!(
                busy = row.busy,
                log_frames = row.log,
                "checkpoint busy; skipping"
            );
        }
        Ok(row) => {
            debug!(
                busy = row.busy,
                log_frames = row.log,
                "final WAL checkpoint completed"
            );
        }
        Err(error) => {
            debug!(error = %error, "failed to run final WAL checkpoint");
        }
    }
}

/// Check if incremental vacuum is enabled via env var FORGE_INCREMENTAL_VACUUM.
/// Defaults to enabled (true) if not set.
fn is_incremental_vacuum_enabled() -> bool {
    match std::env::var("FORGE_INCREMENTAL_VACUUM") {
        Ok(val) => !matches!(val.as_str(), "0" | "false" | "no" | "off"),
        Err(_) => true, // Default: enabled
    }
}

/// Run an incremental vacuum to reclaim freed pages and return them to the OS.
/// Non-fatal: logs errors and continues if vacuum fails.
fn run_incremental_vacuum(connection: &mut SqliteConnection) {
    match connection.batch_execute("PRAGMA incremental_vacuum;") {
        Ok(()) => {
            debug!("incremental_vacuum completed successfully");
        }
        Err(error) => {
            debug!(error = %error, "incremental_vacuum failed (non-fatal, will retry in next checkpoint cycle)");
        }
    }
}
