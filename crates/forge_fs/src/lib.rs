//! # ForgeFS
//!
//! A file system abstraction layer that standardizes error handling for file
//! operations.
//!
//! ForgeFS wraps tokio's filesystem operations with consistent error context
//! using anyhow::Context. Each method provides standardized error messages in
//! the format "Failed to [operation] [path]", ensuring uniform error reporting
//! throughout the application while preserving the original error cause.

mod binary_detection;
mod error;
mod file_size;
mod is_binary;
mod meta;
mod read;
mod read_range;
mod write;

pub use crate::binary_detection::is_binary;
pub use crate::error::Error;

/// ForgeFS provides a standardized interface for file system operations
/// with consistent error handling.
#[derive(Debug)]
pub struct ForgeFS;

impl ForgeFS {
    /// Computes a SHA-256 hash of the given string content.
    pub(crate) fn compute_hash(content: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hex::encode(hasher.finalize())
    }
}
