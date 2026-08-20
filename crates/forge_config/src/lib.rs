mod auto_dump;
mod compact;
mod config;
mod decimal;
mod error;
mod http;
mod legacy;
mod model;
mod output;
mod percentage;
mod reader;
mod reasoning;
mod retry;
mod writer;

pub use auto_dump::*;
pub use compact::*;
pub use config::*;
pub use decimal::*;
pub use error::Error;
pub use http::*;
pub use model::*;
pub use output::*;
pub use percentage::*;
pub use reader::ConfigReader;
pub use reasoning::*;
pub use retry::*;
pub use writer::*;

/// Returns the path to the primary TOML config file (`~/.forge/.forge.toml`).
pub fn config_path() -> std::path::PathBuf {
    ConfigReader::config_path()
}

/// Version information for the running binary.
///
/// Reads the `APP_VERSION` build-time environment variable (injected by the
/// release pipeline), falling back to the crate version for local dev builds.
/// Mirrors `forge_tracker::VERSION` so downstream crates do not need to
/// depend on the tracker just to report a version.
pub const VERSION: &str = match option_env!("APP_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// Fork-owned release repository used by the updater (`forge_main`) and by
/// `heliosdoctor` diagnostics (`forge_services`). Centralizing it here avoids
/// a dependency edge from `forge_services` back to `forge_main`.
pub const DEFAULT_UPDATE_REPO: &str = "KooshaPari/forgecode";

/// A `Result` type alias for this crate's [`Error`] type.
pub type Result<T> = std::result::Result<T, Error>;
