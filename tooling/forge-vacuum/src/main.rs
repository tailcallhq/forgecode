use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::Parser;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(
    name = "forge-vacuum",
    about = "Phenotype-org tooling for safe SQLite vacuum maintenance in the forgecode project.",
    version
)]
struct Args {
    /// SQLite database path.
    #[arg(long, default_value = "~/forge/.forge.db")]
    db_path: PathBuf,

    /// Backup directory.
    #[arg(long)]
    backup_dir: Option<PathBuf>,

    /// Preflight only; report actions without writing.
    #[arg(long)]
    dry_run: bool,

    /// Skip backups before vacuuming.
    #[arg(long)]
    skip_backup: bool,

    /// Minimum free space in MB.
    #[arg(long, default_value_t = 8192)]
    min_free_mb: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FtsMode {
    ExternalContent,
    Contentful,
    Missing,
}

fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .compact()
        .init();

    if let Err(err) = run() {
        eprintln!("forge-vacuum: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let db_path = expand_tilde(&args.db_path)?;
    let backup_dir = match args.backup_dir {
        Some(path) => expand_tilde(&path)?,
        None => db_path
            .parent()
            .map(Path::to_path_buf)
            .context("database path has no parent directory")?,
    };

    let db_size = file_size(&db_path).with_context(|| format!("failed to stat {}", db_path.display()))?;
    let wal_path = sibling_path(&db_path, "-wal");
    let shm_path = sibling_path(&db_path, "-shm");
    let wal_size = file_size_optional(&wal_path)?;
    let shm_size = file_size_optional(&shm_path)?;
    let total_size = db_size + wal_size.unwrap_or(0) + shm_size.unwrap_or(0);

    let free_mb = free_space_mb(&db_path)?;
    if free_mb < args.min_free_mb {
        bail!(
            "refusing to run: free space is {} MB, below the required {} MB on {}",
            free_mb,
            args.min_free_mb,
            db_path.display()
        );
    }

    let holding_pids = open_pids(&[db_path.clone(), wal_path.clone(), shm_path.clone()])?;
    if !holding_pids.is_empty() {
        bail!(
            "refusing to run: database files are open by processes {:?}. close the app before retrying.",
            holding_pids
        );
    }

    let conn = open_connection(&db_path)?;
    let fts_mode = detect_fts_mode(&conn)?;
    info!(
        db = %db_path.display(),
        backup_dir = %backup_dir.display(),
        dry_run = args.dry_run,
        fts_mode = ?fts_mode,
        db_bytes = db_size,
        wal_bytes = wal_size.unwrap_or(0),
        shm_bytes = shm_size.unwrap_or(0),
        total_bytes = total_size,
        free_mb = free_mb,
        "preflight complete"
    );

    if args.dry_run {
        println!(
            "dry-run: would run integrity check, backup, vacuum, fts refresh, optimize, final integrity check"
        );
        println!("dry-run: detected fts mode: {:?}", fts_mode);
        return Ok(());
    }

    if !args.skip_backup {
        let started = Instant::now();
        let backup_root = backup_dir;
        fs::create_dir_all(&backup_root).with_context(|| {
            format!("failed to create backup directory {}", backup_root.display())
        })?;
        backup_database_files(&db_path, &backup_root)?;
        info!("backup completed in {:?}", started.elapsed());
    } else {
        warn!("backup skipped by flag");
    }

    let started = Instant::now();
    quick_check(&conn)?;
    info!("initial integrity check completed in {:?}", started.elapsed());

    let started = Instant::now();
    run_vacuum(&conn)?;
    info!("vacuum completed in {:?}", started.elapsed());

    let started = Instant::now();
    refresh_fts(&conn, fts_mode)?;
    info!("fts refresh completed in {:?}", started.elapsed());

    let started = Instant::now();
    optimize_fts(&conn)?;
    info!("fts optimize completed in {:?}", started.elapsed());

    let started = Instant::now();
    quick_check(&conn)?;
    info!("final integrity check completed in {:?}", started.elapsed());

    let final_db_size = file_size(&db_path)?;
    let final_wal_size = file_size_optional(&wal_path)?.unwrap_or(0);
    let final_shm_size = file_size_optional(&shm_path)?.unwrap_or(0);
    let final_total = final_db_size + final_wal_size + final_shm_size;
    let reclaimed = total_size.saturating_sub(final_total);
    println!("before_bytes={total_size} after_bytes={final_total} reclaimed_bytes={reclaimed}");

    Ok(())
}

fn expand_tilde(path: &Path) -> Result<PathBuf> {
    let path_str = path.to_string_lossy();
    if let Some(rest) = path_str.strip_prefix("~/") {
        let home = dirs::home_dir().context("failed to resolve home directory")?;
        Ok(home.join(rest))
    } else if path_str == "~" {
        dirs::home_dir().context("failed to resolve home directory")
    } else {
        Ok(path.to_path_buf())
    }
}

fn file_size(path: &Path) -> Result<u64> {
    Ok(fs::metadata(path)?.len())
}

fn file_size_optional(path: &Path) -> Result<Option<u64>> {
    match fs::metadata(path) {
        Ok(meta) => Ok(Some(meta.len())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

fn free_space_mb(path: &Path) -> Result<u64> {
    let output = Command::new("df")
        .arg("-Pm")
        .arg(path)
        .output()
        .context("failed to run df")?;
    if !output.status.success() {
        bail!("df failed for {}", path.display());
    }
    let stdout = String::from_utf8(output.stdout)?;
    let line = stdout
        .lines()
        .nth(1)
        .context("unexpected df output")?;
    let free = line
        .split_whitespace()
        .nth(3)
        .context("unexpected df output columns")?
        .parse::<u64>()?;
    Ok(free)
}

fn open_pids(paths: &[PathBuf]) -> Result<Vec<i32>> {
    let mut pids = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let output = Command::new("lsof")
            .arg("-t")
            .arg("--")
            .arg(path)
            .output()
            .with_context(|| format!("failed to run lsof for {}", path.display()))?;
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8(output.stdout)?;
        for pid in stdout.lines().filter_map(|line| line.trim().parse::<i32>().ok()) {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    Ok(pids)
}

fn open_connection(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("failed to open sqlite db {}", path.display()))
}

fn detect_fts_mode(conn: &Connection) -> Result<FtsMode> {
    let ddl: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='conversations_fts'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(ddl) = ddl else {
        return Ok(FtsMode::Missing);
    };
    if ddl.contains("content=") {
        Ok(FtsMode::ExternalContent)
    } else {
        Ok(FtsMode::Contentful)
    }
}

fn quick_check(conn: &Connection) -> Result<()> {
    let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if result != "ok" {
        bail!("quick_check failed: {result}");
    }
    Ok(())
}

fn run_vacuum(conn: &Connection) -> Result<()> {
    conn.execute_batch("VACUUM;")?;
    Ok(())
}

fn refresh_fts(conn: &Connection, mode: FtsMode) -> Result<()> {
    match mode {
        FtsMode::ExternalContent => {
            conn.execute_batch("INSERT INTO conversations_fts(conversations_fts) VALUES('rebuild');")?;
        }
        FtsMode::Contentful => {
            conn.execute_batch(
                "INSERT INTO conversations_fts(conversations_fts) VALUES('delete-all');",
            )?;
            conn.execute_batch(
                "INSERT INTO conversations_fts(conversation_id, title, content, cwd)
                 SELECT conversation_id, title, content, cwd
                 FROM conversations
                 WHERE context IS NOT NULL;",
            )?;
        }
        FtsMode::Missing => {}
    }
    Ok(())
}

fn optimize_fts(conn: &Connection) -> Result<()> {
    conn.execute_batch("INSERT INTO conversations_fts(conversations_fts) VALUES('optimize');")?;
    Ok(())
}

fn backup_database_files(db_path: &Path, backup_root: &Path) -> Result<()> {
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ");
    let backup_name = format!(
        "{}.{}",
        db_path
            .file_name()
            .and_then(|name| name.to_str())
            .context("db path missing file name")?,
        ts
    );
    let backup_dir = backup_root.join(backup_name);
    fs::create_dir_all(&backup_dir)?;

    copy_if_present(db_path, &backup_dir.join(db_path.file_name().context("missing db file name")?))?;
    copy_if_present(
        &sibling_path(db_path, "-wal"),
        &backup_dir.join(
            sibling_path(db_path, "-wal")
                .file_name()
                .context("missing wal file name")?,
        ),
    )?;
    copy_if_present(
        &sibling_path(db_path, "-shm"),
        &backup_dir.join(
            sibling_path(db_path, "-shm")
                .file_name()
                .context("missing shm file name")?,
        ),
    )?;
    Ok(())
}

fn copy_if_present(src: &Path, dst: &Path) -> Result<()> {
    if src.exists() {
        fs::copy(src, dst).with_context(|| format!("failed to copy {} to {}", src.display(), dst.display()))?;
    }
    Ok(())
}
