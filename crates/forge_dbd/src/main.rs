use std::path::PathBuf;

use anyhow::Result;
use forge_dbd::server::DbServer;
use tracing::info;

/// Version string reported by `--version`; mirrors the workspace release
/// version (Cargo.toml `version.workspace = true`).
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Resolves the daemon's socket path.
///
/// Mirrors [`forge_domain::Environment::dbd_socket_path`]:
/// 1. `FORGE_DBD_SOCKET` environment variable, if set — so a spawned daemon can
///    bind where the client expects it.
/// 2. Otherwise the default `~/.forge/.forge.db.sock`.
fn socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("FORGE_DBD_SOCKET") {
        return PathBuf::from(path);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".forge").join(".forge.db.sock")
}

/// Resolves the daemon's write database path.
///
/// Mirrors [`forge_domain::Environment::write_database_path`]:
/// 1. `FORGE_WRITE_DB_PATH` environment variable, if set.
/// 2. Otherwise the split-DB default `~/.forge/.forge.writes.db`, keeping the
///    legacy `~/.forge/.forge.db` untouched for the read-side UNION.
fn db_path() -> PathBuf {
    if let Ok(path) = std::env::var("FORGE_WRITE_DB_PATH") {
        return PathBuf::from(path);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".forge").join(".forge.writes.db")
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    // No-arg flags: the daemon's sole job is serving, so --version/-V and
    // --help return immediately instead of binding a socket.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("forge_dbd {VERSION}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("forge_dbd — SQLite single-writer daemon for forge conversation storage");
        println!("Usage: forge_dbd [--version|-V] [--help|-h]");
        println!("Env:   FORGE_WRITE_DB_PATH (write DB), FORGE_DBD_SOCKET (socket path)");
        return Ok(());
    }

    let socket_path = socket_path();
    let db_path = db_path();
    info!(socket = %socket_path.display(), "starting forge-dbd");

    let server = DbServer::new(socket_path, db_path);
    server.run().await
}
