//! Tmux backend for the [`MuxBridge`] trait.
//!
//! Uses `tmux list-sessions` and `tmux list-windows` (with `-F` format
//! flags) to enumerate active sessions and their windows.  All commands
//! are driven through [`tokio::process::Command`].

use bstr::ByteSlice;
use futures::future::try_join_all;
use tokio::process::Command;

use crate::{MuxBridge, MuxError, MuxSession, MuxWindow};

/// Bridge that shells out to the `tmux` binary.
///
/// # Example
///
/// ```no_run
/// use forge_mux::MuxBridge;
/// use forge_mux::tmux::TmuxBridge;
///
/// # async fn run() -> Result<(), forge_mux::MuxError> {
/// let bridge = TmuxBridge::new();
/// let sessions = bridge.sessions().await?;
/// println!("Active sessions: {sessions:?}");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Default)]
pub struct TmuxBridge;

impl TmuxBridge {
    /// Create a new [`TmuxBridge`].
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl MuxBridge for TmuxBridge {
    /// Enumerate all tmux sessions, fetching windows for each.
    async fn sessions(&self) -> Result<Vec<MuxSession>, MuxError> {
        let raw = run_tmux(&["list-sessions", "-F", "#{session_id}\t#{session_name}"]).await?;
        let sessions = parse_sessions(&raw)?;

        // Fetch windows for every session in parallel.
        let windows_futs: Vec<_> = sessions.iter().map(|s| fetch_windows(&s.name)).collect();

        let all_windows: Vec<Vec<MuxWindow>> = try_join_all(windows_futs).await?;

        // Zip windows back onto their sessions.
        Ok(sessions
            .into_iter()
            .zip(all_windows)
            .map(|(session, windows)| MuxSession { windows, ..session })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Run `tmux` with the given arguments and return trimmed stdout on success.
async fn run_tmux(args: &[&str]) -> Result<String, MuxError> {
    let output = Command::new("tmux").args(args).output().await?;

    if !output.status.success() {
        let stderr = output.stderr.to_str_lossy();
        // tmux returns non-zero when no server is running -> treat as empty.
        if stderr.contains("no server running") {
            return Ok(String::new());
        }
        return Err(MuxError::Parse(format!(
            "tmux exited with {:?}: {}",
            output.status.code(),
            stderr.trim(),
        )));
    }

    let stdout = output.stdout.to_str_lossy().into_owned();
    Ok(stdout.trim().to_string())
}

/// Parse tab-separated session lines into [`MuxSession`] stubs (no windows).
fn parse_sessions(raw: &str) -> Result<Vec<MuxSession>, MuxError> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.split('\t');
        let (Some(id), Some(name)) = (parts.next(), parts.next()) else {
            return Err(MuxError::Parse(format!(
                "expected at least 2 tab-separated fields: {line:?}",
            )));
        };

        sessions.push(MuxSession {
            id: id.to_string(),
            name: name.to_string(),
            windows: Vec::new(),
        });
    }

    Ok(sessions)
}

/// Fetch all windows belonging to a named session via `tmux list-windows`.
async fn fetch_windows(session_name: &str) -> Result<Vec<MuxWindow>, MuxError> {
    let raw = run_tmux(&[
        "list-windows",
        "-t",
        session_name,
        "-F",
        "#{window_id}\t#{window_name}\t#{window_active}",
    ])
    .await?;

    if raw.is_empty() {
        return Ok(Vec::new());
    }

    let mut windows = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.split('\t');
        let (Some(id), Some(name)) = (parts.next(), parts.next()) else {
            return Err(MuxError::Parse(format!(
                "expected at least 2 tab-separated fields: {line:?}",
            )));
        };

        let active = parts.next().unwrap_or("0") == "1";
        windows.push(MuxWindow { id: id.to_string(), name: name.to_string(), active });
    }

    Ok(windows)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sessions_empty() {
        let sessions = parse_sessions("").unwrap();
        assert!(sessions.is_empty());

        let sessions = parse_sessions("  \n  \n").unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_parse_sessions_single() {
        let raw = "$0\twork";
        let sessions = parse_sessions(raw).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "$0");
        assert_eq!(sessions[0].name, "work");
        assert!(sessions[0].windows.is_empty());
    }

    #[test]
    fn test_parse_sessions_multiple() {
        let raw = "$0\twork\n$1\tpersonal\n$2\tcode";
        let sessions = parse_sessions(raw).unwrap();
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].name, "work");
        assert_eq!(sessions[1].name, "personal");
        assert_eq!(sessions[2].name, "code");
    }

    #[test]
    fn test_parse_sessions_too_few_fields() {
        let err = parse_sessions("incomplete_line").unwrap_err();
        match err {
            MuxError::Parse(_) => {} // expected
            other => panic!("expected Parse error, got {other}"),
        }
    }

    #[test]
    fn test_parse_sessions_trailing_newline() {
        let raw = "$0\twork\n";
        let sessions = parse_sessions(raw).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "work");
    }

    #[test]
    fn test_parse_windows_empty() {
        // fetch_windows is async; test the parser logic inline.
        let raw = "";

        // If empty input → empty vector (simulate what fetch_windows does)
        let windows = if raw.is_empty() {
            Vec::new()
        } else {
            let mut w = Vec::new();
            for line in raw.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    let parts: Vec<&str> = line.split('\t').collect();
                    let active = parts.get(2).copied().unwrap_or("0") == "1";
                    w.push(MuxWindow {
                        id: parts[0].to_string(),
                        name: parts[1].to_string(),
                        active,
                    });
                }
            }
            w
        };
        assert!(windows.is_empty());
    }

    #[test]
    fn test_parse_windows() {
        // Simulated tmux list-windows -F output (tab-separated).
        let raw = "@0\teditor\t1\n@1\tterminal\t0\n@2\tmonitor\t1";
        let mut windows = Vec::new();
        for line in raw.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            let active = parts.get(2).copied().unwrap_or("0") == "1";
            windows.push(MuxWindow {
                id: parts[0].to_string(),
                name: parts[1].to_string(),
                active,
            });
        }

        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].id, "@0");
        assert_eq!(windows[0].name, "editor");
        assert!(windows[0].active);

        assert_eq!(windows[1].id, "@1");
        assert_eq!(windows[1].name, "terminal");
        assert!(!windows[1].active);

        assert_eq!(windows[2].id, "@2");
        assert_eq!(windows[2].name, "monitor");
        assert!(windows[2].active);
    }

    #[test]
    fn test_parse_windows_no_active_field() {
        let raw = "@0\teditor";
        let parts: Vec<&str> = raw.split('\t').collect();
        let active = parts.get(2).copied().unwrap_or("0") == "1";
        assert!(!active, "default should be inactive");
    }

    #[test]
    fn test_run_tmux_not_found_io_error() {
        // We cannot easily test the process-level error from here,
        // but verify that the error conversion works at the type level.
        let io: MuxError = std::io::Error::new(std::io::ErrorKind::NotFound, "tmux").into();
        assert!(matches!(io, MuxError::Io(_)));
    }
}
