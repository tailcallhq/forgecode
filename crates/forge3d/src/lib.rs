//! `forge3d` — the shared daemon coordinating forgecode agents.
//!
//! PR-6 ships the long-running daemon that hosts:
//!
//! * the **agent registry** ([`registry::AgentRegistry`]) — a 60-second
//!   lease table of every connected forgecode process;
//! * the **drift-detection store** ([`store::Store`]) — a SQLite
//!   database of observed overlap events and operator-applied
//!   overrides;
//! * the **JSON-RPC server** ([`server::Server`]) — a Unix-domain-socket
//!   listener that accepts framed requests from CLI / shell-plugin
//!   clients and dispatches them to the registry and the store.
//!
//! All public types live in [`lib.rs`](crate); submodules hold
//! implementation details and are not part of the supported surface.
//!
//! ## Wire protocol
//!
//! Each frame on the Unix socket is encoded as a 4-byte big-endian
//! length header followed by UTF-8 JSON. The JSON payload follows
//! JSON-RPC 2.0 with one extension: a top-level `method == "notify"`
//! envelope for server-pushed events (e.g. `drift.alert`).
//!
//! ## Defaults
//!
//! If [`config::ForgeConfig::from_env`] is used, paths follow XDG
//! conventions:
//!
//! * socket: `${XDG_RUNTIME_DIR:-/tmp}/forge3/daemon.sock`
//! * database: `$HOME/.forge/drift.sqlite`
//! * lease: 60 seconds
//! * drift tier: T1 (hash + word distance, no embeddings)
//!
//! ## Quick start
//!
//! ```no_run
//! use forge3d::{ForgeConfig, Server};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let cfg = ForgeConfig::from_env();
//! let mut server = Server::start(cfg).await?;
//! // ... accept loop runs until `server.shutdown().await` ...
//! server.shutdown().await;
//! # Ok(()) }
//! ```

#![deny(missing_debug_implementations)]
#![warn(unreachable_pub)]

pub mod config;
pub mod ipc;
pub mod registry;
pub mod server;
pub mod store;

pub use config::ForgeConfig;
pub use ipc::{RpcError, RpcMessage, UnixSocket};
pub use registry::{AgentEntry, AgentId, AgentRegistry, Lane, Lease, RegistryError};
pub use server::Server;
pub use store::{DriftEvent, Store, StoreError};