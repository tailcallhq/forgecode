//! `forge logs` — stream or list forge log files.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

use crate::cli::LogsArgs;

/// Entry point called from the CLI handler.
pub async fn run(args: LogsArgs, log_dir: PathBuf) -> Result<()> {
    if args.list {
        list(&log_dir).await
    } else {
        let file = match args.file {
            Some(path) => path,
            None => latest(&log_dir).await?,
        };
        tail(&file, args.lines, args.no_follow).await
    }
}

/// Collects every regular file in `log_dir` as `(mtime, path)` pairs.
///
/// Uses a single `entry.metadata()` call per entry — one syscall covers both
/// the `is_file()` check and the `modified()` timestamp.
async fn collect_files(log_dir: &Path) -> Result<Vec<(SystemTime, PathBuf)>> {
    let mut entries = tokio::fs::read_dir(log_dir).await.with_context(|| {
        format!(
            "Log directory not found: {}. Run forge at least once to generate logs.",
            log_dir.display()
        )
    })?;

    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        // Single metadata call per entry covers both is_file() and modified().
        if let Ok(meta) = entry.metadata().await
            && meta.is_file()
        {
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            files.push((mtime, entry.path()));
        }
    }
    Ok(files)
}

/// Returns the path of the most recently modified file inside `log_dir`.
///
/// Uses a linear O(n) scan rather than sorting — we only need the maximum.
async fn latest(log_dir: &Path) -> Result<PathBuf> {
    let files = collect_files(log_dir).await?;
    files
        .into_iter()
        .max_by_key(|(mtime, _)| *mtime)
        .map(|(_, p)| p)
        .ok_or_else(|| anyhow::anyhow!("No log files found in {}", log_dir.display()))
}

/// Lists all log files in `log_dir`, newest first, one path per line on stdout.
async fn list(log_dir: &Path) -> Result<()> {
    let mut files = collect_files(log_dir).await?;
    files.sort_unstable_by_key(|(mtime, _)| *mtime);
    for (_, path) in files.iter().rev() {
        println!("{}", path.display());
    }
    Ok(())
}

/// Spawns `tail` asynchronously, inheriting stdout/stderr.
async fn tail(log_file: &Path, lines: usize, no_follow: bool) -> Result<()> {
    let mut cmd = tokio::process::Command::new("tail");
    cmd.arg(format!("-n{lines}"));
    if !no_follow {
        cmd.arg("-f");
    }
    cmd.arg(log_file);

    let status = cmd.status().await.with_context(|| {
        format!(
            "Failed to run tail on {}. Is `tail` installed?",
            log_file.display()
        )
    })?;

    if !status.success() {
        anyhow::bail!("tail exited with status {}", status.code().unwrap_or(-1));
    }

    Ok(())
}
