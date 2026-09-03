//! forge3d binary — daemon entry point.
//!
//! Starts the forge3d daemon: acquires an exclusive pidfile+flock slot, spins
//! up the drift detector, and begins serving JSON-RPC requests over a Unix
//! domain socket.
//!
//! # Usage
//!
//! ```ignore
//! forge3d \
//!     --drift-dir /var/lib/forge3d/drift \
//!     --socket-path /var/run/forge3d/forge3d.sock \
//!     --pidfile-dir /var/run/forge3d \
//!     [--forge3-client /usr/local/bin/forge3_client]
//! ```
//!
//! If the daemon is already running and `--forge3-client` is supplied, the
//! binary delegates a `forge3_client ping` to the existing instance and exits.

use std::path::PathBuf;
use std::process;
#[cfg(unix)]
use std::sync::Arc;

use clap::Parser;
#[cfg(unix)]
use forge_drift::{DriftConfig, DriftDetector, DriftIndex};
#[cfg(unix)]
use forge3d::pidfile::PidFile;
#[cfg(unix)]
use forge3d::server::Server;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
#[cfg(unix)]
use tokio_util::sync::CancellationToken;

/// Forge3 daemon — agent registry and drift detection over UDS.
///
/// Acquires an exclusive pidfile+flock slot, starts the drift detector, and
/// serves JSON-RPC requests over a Unix domain socket. Exits cleanly on
/// SIGTERM or SIGINT.
#[derive(Parser, Debug)]
#[command(
    name = "forge3d",
    version = env!("CARGO_PKG_VERSION"),
    about = "Forge3 daemon — agent registry and drift detection over UDS",
    long_about = "Forge3 daemon: agent registry and drift detection.\n\n\
        On startup the daemon:\n\
          1. Acquires an exclusive pidfile+flock in --pidfile-dir.\n\
          2. Initialises an in-memory drift detector (T0/Alert mode).\n\
          3. Binds a Unix domain socket at --socket-path and begins serving\n\
             JSON-RPC 2.0 requests (agent.register, agent.heartbeat,\n\
             agent.deregister, agent.list, drift.observe, drift.override).\n\n\
        If another instance already holds the lock and --forge3-client is\n\
        supplied, the binary delegates a `forge3_client ping` to the running\n\
        daemon and exits with code 0.  If no --forge3-client is given in that\n\
        situation, the binary exits silently with code 0 as well.\n\n\
        The daemon shuts down gracefully on SIGTERM or SIGINT, draining\n\
        in-flight connections before releasing the socket and pidfile."
)]
struct Args {
    /// Directory used for drift storage.
    ///
    /// Currently reserved for future persistent state. The directory must
    /// exist and be writable by the daemon process.
    #[arg(long)]
    drift_dir: PathBuf,

    /// Path to the Unix domain socket the daemon listens on.
    ///
    /// Any stale socket file from a previous run is removed before the daemon
    /// binds. Ensure the parent directory exists and is writable.
    #[arg(long)]
    socket_path: PathBuf,

    /// Directory for the PID file and exclusive daemon lock.
    ///
    /// The daemon creates `forge3d.pid` and acquires an `flock` in this
    /// directory. If the lock is already held by another process the daemon
    /// either delegates a ping (see --forge3-client) or exits with code 0.
    #[arg(long)]
    pidfile_dir: PathBuf,

    /// Path to the `forge3_client` binary for daemon-already-running checks.
    ///
    /// When provided and the daemon is already running (lock held), the binary
    /// executes `forge3_client ping` against the existing instance to confirm
    /// liveness, prints the result, and exits with code 0.  If the daemon is
    /// not running this argument has no effect and the current process becomes
    /// the daemon.
    #[arg(long)]
    forge3_client: Option<String>,
}

#[tokio::main]
async fn main() {
    #[cfg(not(unix))]
    {
        let _args = Args::parse();
        eprintln!("forge3d is only supported on Unix platforms");
        process::exit(1);
    }

    #[cfg(unix)]
    {
        let args = Args::parse();
        run_unix(args).await;
    }
}

#[cfg(unix)]
async fn run_unix(args: Args) {
    // Initialise a minimal tracing subscriber so the daemon's own log
    // messages (from `server.rs`, `pidfile.rs`, etc.) are visible.
    tracing_subscriber::fmt::init();

    let pid = process::id();

    // ------------------------------------------------------------------
    // 1. Acquire exclusive daemon slot (pidfile + flock)
    // ------------------------------------------------------------------
    let pidfile = match PidFile::acquire(&args.pidfile_dir, pid) {
        Ok(pf) => {
            tracing::info!(pid, dir = %args.pidfile_dir.display(), "acquired daemon slot");
            pf
        }
        Err(forge3d::error::Forge3Error::AlreadyRunning)
        | Err(forge3d::error::Forge3Error::LockHeld { .. }) => {
            // Another instance is already running. If the caller provided a
            // forge3_client binary, delegate a `ping` to confirm liveness,
            // then exit cleanly.
            if let Some(ref client_bin) = args.forge3_client {
                let status = std::process::Command::new(client_bin).arg("ping").status();
                match status {
                    Ok(s) if s.success() => {
                        tracing::info!("forge3d is already running — ping OK");
                    }
                    Ok(s) => {
                        eprintln!("forge3d: {client_bin} ping exited with {s}");
                    }
                    Err(e) => {
                        eprintln!("forge3d: failed to run {client_bin} ping: {e}");
                    }
                }
            }
            process::exit(0);
        }
        Err(e) => {
            eprintln!("forge3d: failed to acquire pidfile: {e}");
            process::exit(1);
        }
    };

    // ------------------------------------------------------------------
    // 2. Build the drift detector (in-memory, T0 / Alert mode by default)
    // ------------------------------------------------------------------
    let drift_config = DriftConfig::default();
    let drift_index = Arc::new(DriftIndex::new());
    let drift_detector = DriftDetector::new(drift_config, drift_index, None);

    // ------------------------------------------------------------------
    // 3. Build the server
    // ------------------------------------------------------------------
    let server = Arc::new(
        Server::new()
            .with_pidfile(pidfile)
            .with_drift_detector(drift_detector),
    );
    let shutdown = CancellationToken::new();

    // ------------------------------------------------------------------
    // 4. Spawn the serve loop in a background task so we can listen for shutdown
    //    signals concurrently.
    // ------------------------------------------------------------------
    let serve_handle = {
        let server = server.clone();
        let socket_path = args.socket_path.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            if let Err(e) = server.serve(&socket_path, shutdown).await {
                eprintln!("forge3d: server error: {e}");
                process::exit(1);
            }
        })
    };

    // ------------------------------------------------------------------
    // 5. Wait for SIGTERM or SIGINT for a graceful shutdown.
    // ------------------------------------------------------------------
    let mut sigterm =
        signal(SignalKind::terminate()).expect("failed to create SIGTERM signal handler");
    let mut sigint =
        signal(SignalKind::interrupt()).expect("failed to create SIGINT signal handler");

    tokio::select! {
        _ = sigterm.recv() => tracing::info!("received SIGTERM"),
        _ = sigint.recv() => tracing::info!("received SIGINT"),
    }

    tracing::info!("shutting down");

    // Drop the server (and thus the PidFile) to release the flock before the
    // serve task is aborted.  The socket file is cleaned up on next start.
    shutdown.cancel();
    drop(server);
    serve_handle.abort();
}
