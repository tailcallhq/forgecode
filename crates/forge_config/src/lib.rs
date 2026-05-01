mod auto_dump;
mod compact;
mod config;
mod decimal;
mod error;
mod formatter; // NEW: Tool output formatter
mod http;
mod legacy;
mod model;
mod output; // NEW: Output formatting settings
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
pub use formatter::*; // NEW: Export ToolOutputFormatter
pub use http::*;
pub use legacy::*;
pub use model::*;
pub use output::{OutputSettings, Verbosity}; // NEW: Export OutputSettings, Verbosity
pub use percentage::*;
pub use reader::*;
pub use reasoning::*;
pub use retry::*;
pub use writer::*;

/// A `Result` type alias for this crate's [`Error`] type.
pub type Result<T> = std::result::Result<T, Error>;
