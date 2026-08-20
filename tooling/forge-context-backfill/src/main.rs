/// forge-context-backfill: Batch-compress existing uncompressed conversation rows
///
/// This tool safely migrates existing uncompressed context blobs (is_compressed=0) in
/// the forge conversation database to zstd-compressed format (is_compressed=1).
///
/// Safety guarantees:
/// - Preflight check: refuses if forge processes hold the DB (lsof)
/// - Disk check: refuses if < (db_size + 1GB) free space
/// - Backup first: automatic timestamped backup (skippable with --skip-backup + warning)
/// - Batched + resumable: processes in transactions, idempotent (skips already-compressed rows)
/// - Lossless verification: round-trip verify each row before write
/// - Vacuum option: --vacuum runs full VACUUM + converts DB to INCREMENTAL auto_vacuum
///
/// Usage (dry-run by default):
///   cargo run -- --db-path ~/forge/.forge.db
///
/// Usage (apply compression):
///   cargo run -- --db-path ~/forge/.forge.db --apply --yes
///
/// Usage (with full vacuum):
///   cargo run -- --db-path ~/forge/.forge.db --apply --yes --vacuum
use anyhow::{anyhow, Result};
use clap::Parser;
use humansize::{format_size, BINARY};
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{error, info, warn};

mod codec;
mod db;
mod proc;
mod report;

use crate::db::Database;
use crate::proc::ProcessCheck;
use crate::report::Report;

#[derive(Parser, Debug)]
#[command(name = "forge-context-backfill")]
#[command(about = "Batch-compress existing uncompressed conversation rows")]
struct Args {
    /// Path to the forge database
    #[arg(long, default_value = "~/.forge.db")]
    db_path: String,

    /// Enable actual compression (default: dry-run)
    #[arg(long)]
    apply: bool,

    /// Assume yes to all confirmations
    #[arg(long)]
    yes: bool,

    /// Process rows in batches of this size
    #[arg(long, default_value = "200")]
    batch_size: usize,

    /// Directory for backup (default: same as db)
    #[arg(long)]
    backup_dir: Option<String>,

    /// Skip automatic backup (NOT RECOMMENDED)
    #[arg(long)]
    skip_backup: bool,

    /// Run full VACUUM after compression to reclaim space + convert to incremental
    #[arg(long)]
    vacuum: bool,
}

fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("forge_context_backfill=info".parse()?),
        )
        .init();

    let args = Args::parse();

    // Expand ~ in path
    let db_path = shellexpand::tilde(&args.db_path).to_string();
    let db_path = PathBuf::from(db_path);

    info!(
        "forge-context-backfill starting (dry_run={})",
        !args.apply
    );

    // DRY RUN: Skip safety gates (read-only operations)
    if !args.apply {
        info!("DRY RUN MODE: Opening database in read-only mode (no safety gates needed)");
    } else {
        // SAFETY GATE 1: Check for running processes (apply mode only)
        info!("SAFETY GATE 1: Checking for running forge processes...");
        let proc_check = ProcessCheck::check(&db_path)?;
        if proc_check.has_holders() {
            error!(
                "SAFETY GATE 1 FAILED: {} process(es) hold the database",
                proc_check.count()
            );
            eprintln!(
                "\nREFUSED: Cannot backfill while forge processes hold the database.\n\n\
                 Holding processes (PIDs):\n{}\n\
                 Please close these processes or wait for them to release the database.\n\
                 Run: lsof -t {} | xargs ps -o pid,cmd\n",
                proc_check.format_pids(),
                db_path.display()
            );
            return Err(anyhow!("Preflight check failed: database is held by active processes"));
        }
        info!("✓ No processes hold the database");

        // SAFETY GATE 2: Check disk space (apply mode only)
        info!("SAFETY GATE 2: Checking available disk space...");
        let db_size = fs::metadata(&db_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let required_space = db_size + (1024 * 1024 * 1024); // +1GB buffer
        let available = disk_free(&db_path)?;

        if available < required_space {
            error!(
                "SAFETY GATE 2 FAILED: Insufficient disk space. Required: {}, Available: {}",
                format_size(required_space, BINARY),
                format_size(available, BINARY)
            );
            return Err(anyhow!("Insufficient disk space for backfill"));
        }
        info!(
            "✓ Disk space OK (available: {}, required: {})",
            format_size(available, BINARY),
            format_size(required_space, BINARY)
        );

        // SAFETY GATE 3: Backup (apply mode only)
        info!("SAFETY GATE 3: Backup...");
        if args.skip_backup {
            warn!("⚠ Skipping backup (--skip-backup). This is NOT RECOMMENDED.");
        } else {
            let backup_path = if let Some(dir) = args.backup_dir {
                PathBuf::from(shellexpand::tilde(&dir).to_string())
                    .join(format!(
                        ".forge.db.backup-{}",
                        chrono::Local::now().format("%Y%m%d-%H%M%S")
                    ))
            } else {
                db_path.parent().unwrap_or(Path::new(".")).join(format!(
                    ".forge.db.backup-{}",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                ))
            };

            info!("Creating backup: {}", backup_path.display());
            fs::copy(&db_path, &backup_path)?;
            info!(
                "✓ Backup created: {} ({} bytes)",
                backup_path.display(),
                fs::metadata(&backup_path)?.len()
            );
        }
    }

    // Open database
    info!("Opening database: {}", db_path.display());
    let mut db = if args.apply {
        Database::open(&db_path)?
    } else {
        Database::open_readonly(&db_path)?
    };

    // DRY RUN: Count how many rows would be compressed
    info!("Counting uncompressed rows...");
    let total_rows = db.count_uncompressed_rows()?;
    info!("Found {} uncompressed rows", total_rows);

    if total_rows == 0 {
        info!("✓ All rows are already compressed or database is empty. Nothing to do.");
        return Ok(());
    }

    // Get initial stats
    let initial_stats = db.get_compression_stats()?;
    info!(
        "Initial stats: {} total rows, {} compressed, {} uncompressed",
        initial_stats.total_rows,
        initial_stats.compressed_rows,
        initial_stats.uncompressed_rows
    );

    // Show what WOULD be compressed
    let mut report = Report::new(total_rows);
    info!(
        "DRY RUN: Would compress {} rows",
        total_rows
    );
    if !args.apply {
        eprintln!(
            "\n╔════════════════════════════════════════════════════════════════╗\n\
             ║ DRY RUN: Showing what would be compressed                       ║\n\
             ╠════════════════════════════════════════════════════════════════╣"
        );
        eprintln!("║ Uncompressed rows:        {:>44} ║", total_rows);
        eprintln!(
            "║ Batch size:               {:>44} ║",
            args.batch_size
        );
        eprintln!("║ Operation:                {:>44} ║", if args.apply {
            "APPLY (writing to DB)"
        } else {
            "DRY RUN (no changes)"
        });
        eprintln!(
            "╚════════════════════════════════════════════════════════════════╝\n\
             \n\
             To apply compression, re-run with: --apply --yes\n"
        );
        return Ok(());
    }

    // APPLY MODE: Require explicit --yes
    if !args.yes {
        error!("--apply requires --yes confirmation");
        return Err(anyhow!(
            "--apply requires explicit --yes confirmation to protect against accidental runs"
        ));
    }

    warn!("APPLYING COMPRESSION: This will modify the database");

    // Process rows in batches
    let start = Instant::now();
    let mut batch_num = 0;

    loop {
        batch_num += 1;
        info!(
            "Processing batch {} (offset: {}, batch_size: {})",
            batch_num,
            (batch_num - 1) * args.batch_size,
            args.batch_size
        );

        let compressed_in_batch = db.compress_batch(args.batch_size, &mut report)?;

        if compressed_in_batch == 0 {
            info!("Batch {} returned 0 rows (all compressed or none remaining)", batch_num);
            break;
        }

        info!(
            "✓ Batch {} compressed {} rows",
            batch_num, compressed_in_batch
        );
    }

    let elapsed = start.elapsed();
    info!("✓ Compression complete in {:.2}s", elapsed.as_secs_f64());

    // Get final stats
    let final_stats = db.get_compression_stats()?;
    info!(
        "Final stats: {} total rows, {} compressed, {} uncompressed",
        final_stats.total_rows,
        final_stats.compressed_rows,
        final_stats.uncompressed_rows
    );

    // Close database before vacuum
    drop(db);

    // VACUUM if requested
    if args.vacuum {
        info!("Running full VACUUM to reclaim space and convert to incremental auto_vacuum...");
        let vacuum_start = Instant::now();

        let conn = Connection::open(&db_path)?;
        conn.execute("PRAGMA auto_vacuum = INCREMENTAL;", [])?;
        conn.execute("VACUUM;", [])?;
        conn.close()
            .map_err(|_| anyhow!("Failed to close database after VACUUM"))?;

        let vacuum_elapsed = vacuum_start.elapsed();
        info!(
            "✓ VACUUM complete in {:.2}s",
            vacuum_elapsed.as_secs_f64()
        );

        let db_size_after = fs::metadata(&db_path)?.len();
        info!(
            "Database size: {} → {} (saved: {})",
            format_size(initial_stats.total_size, BINARY),
            format_size(db_size_after, BINARY),
            format_size(
                initial_stats.total_size.saturating_sub(db_size_after),
                BINARY
            )
        );
    }

    // Print final report
    report.print(
        &initial_stats,
        &final_stats,
        elapsed,
    );

    eprintln!(
        "\n╔════════════════════════════════════════════════════════════════╗\n\
         ║ ✓ COMPRESSION COMPLETE                                        ║\n\
         ╚════════════════════════════════════════════════════════════════╝\n"
    );

    Ok(())
}

/// Check available disk space on the filesystem containing the given path
fn disk_free(path: &Path) -> Result<u64> {
    use nix::sys::statvfs::statvfs;
    let stat = statvfs(path)?;
    Ok((stat.blocks_available() as u64) * (stat.block_size() as u64))
}
