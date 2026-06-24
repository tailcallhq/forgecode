//! `ghostty-kit` — a minimal, zero-dependency parser for Ghostty's
//! INI-style configuration files.
//!
//! Ghostty's config format is documented at
//! <https://ghostty.org/docs/config>. This crate implements the subset
//! the forgecode integration needs:
//!
//! * `key = value` lines, with type inference (`bool`, `integer`,
//!   `color`, `string`, comma-separated `list`).
//! * `#` comments (full-line and trailing).
//! * `[section name]` headers.
//! * `config-file = path` includes (with cycle detection).
//! * `$name` and `${name}` variable substitution.
//! * `\` line-continuation for multi-line values.
//!
//! The parser never panics on malformed input — every error is reported
//! with a `PathBuf` and a 1-based line number via [`ConfigError`].

mod config;
mod error;
mod ipc;
mod ipc_request;
mod ipc_response;
mod serialize;
mod value;

pub use config::{
    get, get_section, parse, parse_file, resolve_includes, substitute_variables, ConfigEntry,
    ConfigValue, GhosttyConfig,
};
pub use error::{ConfigError, Result};
pub use ipc::{GhosttyControl, IpcError, ProgressState, Response, WindowSize};

#[doc(hidden)]
pub use config::to_json;
