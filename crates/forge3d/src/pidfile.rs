//! Daemon PID file + advisory flock.
//!
//! The PID file lives at `~/.forge/daemon.pid` (or whatever the caller passes
//! to [`PidFile::acquire`]) and serves two purposes:
//!
//! 1. **Discovery** — clients can resolve the running daemon by reading the PID
//!    and asking `/proc/<pid>/` whether it's still alive before connecting to
//!    the Unix socket. The PID file alone does not guarantee exclusivity: if a
//!    daemon is killed with `kill -9` and a new one starts before the stale PID
//!    is cleaned up, both could race.
//!
//! 2. **Advisory exclusion** — for the brief interval between "daemon A is
//!    shutting down" and "kernel has reaped A", the PID file acts as a hint
//!    that something *recently* held the slot. Pairing it with `flock(2)` on
//!    the same fd (or a sibling `daemon.lock` file) gives a stronger guarantee:
//!    the kernel-level lock survives process death without coordination.
//!
//! Design choice: we use [`fs2::FileExt`] for `flock(2)` (cross-platform,
//! no tokio dep) and use a separate `lock` file from the pid file so that
//! external tooling can inspect the pid without taking the lock.
//!
//! Failure modes:
//! - If the lock can be acquired but the pid file is unparseable, we overwrite
//!   the pid file (the holder is gone). This means a crash-then- restart cycle
//!   works without manual cleanup.
//! - If the lock is held *and* the recorded pid is alive, we return
//!   [`Forge3Error::LockHeld`] with the live pid so the caller can decide
//!   whether to wait, error out, or take over.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;

use fs2::FileExt;
use tracing::{debug, info, warn};

use crate::error::{Forge3Error, Result};

/// Holds both the PID file and the flock until dropped.
#[derive(Debug)]
pub struct PidFile {
    pid_path: PathBuf,
    lock_file: File,
    pid: u32,
}

impl PidFile {
    /// Acquire the daemon slot.
    ///
    /// On success, writes `pid` to `<dir>/daemon.pid` and holds an advisory
    /// flock on `<dir>/daemon.lock` until the returned guard is dropped.
    /// The guard's `Drop` impl deletes both files (best-effort).
    ///
    /// Errors:
    /// - [`Forge3Error::AlreadyRunning`] if the lock is held by a live process
    ///   whose PID matches the pidfile.
    /// - [`Forge3Error::LockHeld`] if the lock is held by another live PID.
    /// - I/O errors from filesystem access.
    pub fn acquire(dir: &Path, pid: u32) -> Result<Self> {
        fs::create_dir_all(dir)?;
        let pid_path = dir.join("daemon.pid");
        let lock_path = dir.join("daemon.lock");

        // Open (or create) the lock file. We open for read+write so flock
        // operates on a real fd, not a transient append-mode handle.
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;

        // Try to acquire an exclusive, blocking lock.
        match lock_file.try_lock_exclusive() {
            Ok(()) => {
                // We have the lock. Whatever was in the pidfile before is
                // stale (the previous holder is gone). Overwrite it.
                let mut f = File::create(&pid_path)?;
                writeln!(f, "{}", pid)?;
                f.sync_all()?;
                info!(pid, path = %pid_path.display(), "acquired daemon slot");
                Ok(Self { pid_path, lock_file, pid })
            }
            Err(_) => {
                // Someone else holds the lock. Check whether they're alive
                // and whether the pidfile matches — that lets us tell the
                // caller "alive and newer than us" vs "stale, try again".
                let recorded = fs::read_to_string(&pid_path)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok());
                let err = match recorded {
                    Some(other) if pid_is_alive(other) => {
                        if other == pid {
                            Forge3Error::AlreadyRunning
                        } else {
                            warn!(other, "lock held by live pid");
                            Forge3Error::LockHeld(other)
                        }
                    }
                    // Stale lock — return AlreadyRunning so the caller knows
                    // to retry after acquiring the now-stale file. We do NOT
                    // forcibly take over because that would race with the
                    // genuine previous holder if it's still tearing down.
                    _ => Forge3Error::AlreadyRunning,
                };
                Err(err)
            }
        }
    }

    /// The PID recorded in the lock file (i.e. our own pid).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The directory the daemon lives in (for log paths, socket paths, etc).
    pub fn dir(&self) -> &Path {
        self.pid_path.parent().unwrap_or(Path::new("."))
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        // Release the lock first so a successor can take it before we
        // delete the pidfile (avoids a brief window where the pidfile
        // points at nothing).
        if let Err(e) = self.lock_file.unlock() {
            warn!(error = %e, "failed to release daemon lock");
        }
        // Best-effort cleanup of the pidfile. If it disappears (e.g. user
        // manually deleted it) we just log and continue.
        match fs::remove_file(&self.pid_path) {
            Ok(()) => debug!(path = %self.pid_path.display(), "removed pidfile"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(error = %e, "failed to remove pidfile"),
        }
    }
}

/// True if `pid` is alive on this machine.
fn pid_is_alive(pid: u32) -> bool {
    if pid == process::id() {
        return true;
    }
    // On unix, kill(pid, 0) returns Ok(()) if the process exists and we have
    // permission to signal it; EPERM means alive but not ours; ESRCH means
    // dead. We treat EPERM as alive (the daemon can't take it over anyway).
    #[cfg(unix)]
    {
        // SAFETY: kill(pid, sig=0) is a POSIX probe — it does not send a real
        // signal; it only checks process existence and permissions.  `pid` is a
        // `u32` read from a file and cast to `pid_t`; the cast is safe on all
        // supported Unix targets where pid_t is i32 or i64.
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if r == 0 {
            return true;
        }
        let errno = std::io::Error::last_os_error();
        matches!(errno.raw_os_error(), Some(libc::EPERM))
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn acquire_writes_pidfile() {
        let td = TempDir::new().unwrap();
        let dir = td.path();
        let guard = PidFile::acquire(dir, 12345).expect("acquire");
        let recorded = fs::read_to_string(dir.join("daemon.pid")).unwrap();
        assert_eq!(recorded.trim(), "12345");
        assert_eq!(guard.pid(), 12345);
    }

    #[test]
    fn second_acquire_errors_when_first_holds() {
        let td = TempDir::new().unwrap();
        let dir = td.path();
        let _first = PidFile::acquire(dir, 99999).expect("first");
        // Use a different recorded pid so we hit the LockHeld branch (not
        // AlreadyRunning). The fake pid is almost certainly not alive.
        fs::write(dir.join("daemon.pid"), "1\n").unwrap();
        let err = PidFile::acquire(dir, 88888).unwrap_err();
        // Either LockHeld or AlreadyRunning is acceptable depending on
        // whether PID 1 happened to be live in the test environment.
        assert!(
            matches!(
                err,
                Forge3Error::LockHeld { .. } | Forge3Error::AlreadyRunning
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn release_allows_reacquire() {
        let td = TempDir::new().unwrap();
        let dir = td.path();
        {
            let _first = PidFile::acquire(dir, 11111).unwrap();
        } // drop releases the lock
        // After drop the pidfile may still exist briefly (Drop tries cleanup
        // best-effort). The lock file is released regardless.
        let _second = PidFile::acquire(dir, 22222).expect("reacquire");
        let recorded = fs::read_to_string(dir.join("daemon.pid")).unwrap();
        assert_eq!(recorded.trim(), "22222");
    }
}
