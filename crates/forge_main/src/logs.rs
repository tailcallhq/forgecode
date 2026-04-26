//! `forge logs` — stream or list forge log files.

use std::path::{Path, PathBuf};

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

/// Returns the path of the most recently modified file inside `log_dir`.
async fn latest(log_dir: &Path) -> Result<PathBuf> {
    let mut entries = tokio::fs::read_dir(log_dir).await.with_context(|| {
        format!(
            "Log directory not found: {}. Run forge at least once to generate logs.",
            log_dir.display()
        )
    })?;

    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if tokio::fs::metadata(&path).await.map(|m| m.is_file()).unwrap_or(false) {
            let mtime = entry
                .metadata()
                .await
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            files.push((mtime, path));
        }
    }

    files.sort_unstable_by_key(|(mtime, _)| *mtime);
    files
        .into_iter()
        .next_back()
        .map(|(_, p)| p)
        .ok_or_else(|| anyhow::anyhow!("No log files found in {}", log_dir.display()))
}

/// Lists all log files in `log_dir`, newest first, one path per line on stdout.
async fn list(log_dir: &Path) -> Result<()> {
    let mut entries = tokio::fs::read_dir(log_dir).await.with_context(|| {
        format!(
            "Log directory not found: {}. Run forge at least once to generate logs.",
            log_dir.display()
        )
    })?;

    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if tokio::fs::metadata(&path).await.map(|m| m.is_file()).unwrap_or(false) {
            let mtime = entry
                .metadata()
                .await
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            files.push((mtime, path));
        }
    }

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
