//! Reusable client + wire protocol for the `forge_dbd` SQLite write daemon.
//!
//! The daemon binary (`src/main.rs`) hosts the server half in a private
//! `server` module. Downstream crates (forge_repo and friends) only need the
//! client and the protocol types, so the server is deliberately kept out of
//! this library's surface: `server` stays bin-only.

pub mod client;
pub mod conversation_storage;
pub mod protocol;
