//! Daemon configuration.
//!
//! [`ForgeConfig`] is the single resolved configuration for a running
//! `forge3d` instance. It is built from environment variables (consulted
//! by [`ForgeConfig::from_env`]) with the following precedence:
//!
//! 1. `FORGE3_SOCKET`, `FORGE3_DB`, `FORGE3_PID`, `FORGE3_LOCK`,
//!    `FORGE3_LEASE_SECS`, `FORGE3_TIER` — environment overrides;
//! 2. Defaults — XDG-aware socket path, `$HOME/.forge/drift.sqlite`,
//!    60-second lease, T1 drift tier.
//!
//! Configuration is infallible: missing variables simply fall back to
//! defaults. The daemon never panics on bad config.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Drift-detection tier. T0 is exact-match only; T1 adds word-distance
/// similarity; T2 (future) adds embeddings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftTier {
    /// Hash equality only. Cheapest, most conservative.
    T0,
    /// Hash equality + word-set Jaccard distance. No embeddings.
    T1,
}

impl DriftTier {
    /// Parse from `&str`. Case-insensitive; defaults to [`T1`](Self::T1)
    /// for any unknown value so a typo can never disable drift
    /// detection silently.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_uppercase().as_str() {
            "T0" | "0" | "HASH" => Self::T0,
            "T2" | "2" | "EMBED" => Self::T1, // T2 not implemented; fall back
            _ => Self::T1,
        }
    }

    /// Stable string representation used in logs and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::T0 => "T0",
            Self::T1 => "T1",
        }
    }
}

/// Default socket location under `XDG_RUNTIME_DIR`.
const DEFAULT_XDG_SOCKET: &str = "forge3/daemon.sock";
/// Hard fallback when `XDG_RUNTIME_DIR` is unset.
const DEFAULT_FALLBACK_SOCKET: &str = "/tmp/forge3/daemon.sock";
/// Default SQLite location under `$HOME`.
const DEFAULT_DB: &str = ".forge/drift.sqlite";
/// Default lease length for agent registrations (seconds).
const DEFAULT_LEASE_SECS: u64 = 60;
/// Default retention window (days).
const DEFAULT_RETENTION_DAYS: u32 = 30;

/// Resolved configuration for a running daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeConfig {
    /// Path the UDS listener binds to.
    pub socket_path: PathBuf,
    /// SQLite database file.
    pub db_path: PathBuf,
    /// File holding the running daemon's PID.
    pub pid_path: PathBuf,
    /// File used for `flock` inter-process exclusion.
    pub lock_path: PathBuf,
    /// Lease length after `agent.register` / `agent.heartbeat`.
    pub lease: Duration,
    /// Retention window for the `drift_events` table (days).
    pub retention_days: u32,
    /// Drift-detection tier.
    pub tier: DriftTier,
}

impl ForgeConfig {
    /// Build a config from environment variables, falling back to
    /// defaults. Never panics.
    pub fn from_env() -> Self {
        let socket_path = env_path("FORGE3_SOCKET").unwrap_or_else(default_socket_path);
        let db_path = env_path("FORGE3_DB").unwrap_or_else(default_db_path);
        let pid_path = env_path("FORGE3_PID").unwrap_or_else(|| with_extension(&socket_path, "pid"));
        let lock_path =
            env_path("FORGE3_LOCK").unwrap_or_else(|| with_extension(&socket_path, "lock"));
        let lease = env_u64("FORGE3_LEASE_SECS")
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_LEASE_SECS));
        let retention_days = env_u32("FORGE3_RETENTION_DAYS").unwrap_or(DEFAULT_RETENTION_DAYS);
        let tier = env_string("FORGE3_TIER")
            .map_or(DriftTier::T1, |s| DriftTier::parse(&s));

        Self {
            socket_path,
            db_path,
            pid_path,
            lock_path,
            lease,
            retention_days,
            tier,
        }
    }

    /// Helper used by tests and `Server::start_at` that need a fully
    /// custom layout. Defaults are used for everything except the two
    /// paths the caller supplies.
    pub fn for_paths(socket_path: PathBuf, db_path: PathBuf) -> Self {
        let pid_path = with_extension(&socket_path, "pid");
        let lock_path = with_extension(&socket_path, "lock");
        Self {
            socket_path,
            db_path,
            pid_path,
            lock_path,
            lease: Duration::from_secs(DEFAULT_LEASE_SECS),
            retention_days: DEFAULT_RETENTION_DAYS,
            tier: DriftTier::T1,
        }
    }
}

/// Build the default socket path following XDG conventions.
fn default_socket_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join(DEFAULT_XDG_SOCKET);
    }
    PathBuf::from(DEFAULT_FALLBACK_SOCKET)
}

/// Build the default SQLite path under `$HOME` if available.
fn default_db_path() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(DEFAULT_DB);
    }
    PathBuf::from("/tmp/forge3/drift.sqlite")
}

/// Compute `<socket>.pid` / `<socket>.lock` style paths by appending a
/// new extension. The original file stem is preserved.
fn with_extension(socket_path: &Path, ext: &str) -> PathBuf {
    let parent = socket_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = socket_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("daemon");
    parent.join(format!("{stem}.{ext}"))
}

fn env_string(key: &str) -> Option<String> {
    std::env::var_os(key).map(|v| v.to_string_lossy().into_owned())
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from)
}

fn env_u64(key: &str) -> Option<u64> {
    env_string(key).and_then(|s| s.parse::<u64>().ok())
}

fn env_u32(key: &str) -> Option<u32> {
    env_string(key).and_then(|s| s.parse::<u32>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_parse_round_trips_known_values() {
        assert_eq!(DriftTier::parse("T0"), DriftTier::T0);
        assert_eq!(DriftTier::parse("t1"), DriftTier::T1);
        assert_eq!(DriftTier::parse("0"), DriftTier::T0);
        assert_eq!(DriftTier::parse("garbage"), DriftTier::T1);
        assert_eq!(DriftTier::T1.as_str(), "T1");
    }

    #[test]
    fn with_extension_replaces_stem() {
        let p = with_extension(&PathBuf::from("/var/run/forge3/d.sock"), "pid");
        assert_eq!(p, PathBuf::from("/var/run/forge3/d.pid"));
    }

    #[test]
    fn with_extension_handles_no_extension() {
        let p = with_extension(&PathBuf::from("/tmp/daemon"), "lock");
        assert_eq!(p, PathBuf::from("/tmp/daemon.lock"));
    }

    #[test]
    fn for_paths_derives_pid_and_lock() {
        let cfg = ForgeConfig::for_paths(
            PathBuf::from("/var/run/forge3/d.sock"),
            PathBuf::from("/var/lib/forge3/d.sqlite"),
        );
        assert_eq!(cfg.pid_path, PathBuf::from("/var/run/forge3/d.pid"));
        assert_eq!(cfg.lock_path, PathBuf::from("/var/run/forge3/d.lock"));
        assert_eq!(cfg.db_path, PathBuf::from("/var/lib/forge3/d.sqlite"));
        assert_eq!(cfg.lease, Duration::from_secs(60));
        assert_eq!(cfg.retention_days, 30);
        assert_eq!(cfg.tier, DriftTier::T1);
    }
}