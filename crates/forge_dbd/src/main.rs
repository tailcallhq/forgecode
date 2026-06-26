// Scaffold crate: `client` and parts of `server`/`protocol` are stub APIs that
// the daemon does not yet wire up. Allow dead_code until the real daemon logic
// (Unix-socket serving + client connection) is implemented.
#![allow(dead_code)]

mod client;
mod protocol;
mod server;

use std::path::PathBuf;

use anyhow::Result;
use tracing::info;

fn socket_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".forge").join(".forge.db.sock")
}

fn db_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".forge").join("forge.db")
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let socket_path = socket_path();
    let db_path = db_path();
    info!(socket = %socket_path.display(), "starting forge-dbd");

    let server = server::DbServer::new(socket_path, db_path);
    server.run().await
}
