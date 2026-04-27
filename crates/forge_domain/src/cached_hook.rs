use std::path::PathBuf;

/// A hook script whose content was read into memory at startup.
///
/// The `source` field is kept for logging/diagnostics only. The `content`
/// field holds the full file bytes loaded at construction time and is used
/// for all subsequent executions — no further disk access occurs.
///
/// The actual execution logic (memfd on Linux, temp-file fallback elsewhere)
/// lives in `forge_app`; this struct is defined in `forge_domain` so it can
/// flow through the domain layer without coupling to platform-specific APIs.
#[derive(Clone, Debug)]
pub struct CachedHook {
    /// Original source path — kept for logging / diagnostics only.
    source: PathBuf,
    /// Full file content loaded at construction time.
    content: Vec<u8>,
}

impl CachedHook {
    /// Reads the file at `path` into memory.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read.
    pub fn from_path(path: PathBuf) -> std::io::Result<Self> {
        let content = std::fs::read(&path)?;
        Ok(Self { source: path, content })
    }

    /// Returns the original source path (for diagnostics only).
    pub fn source(&self) -> &std::path::Path {
        &self.source
    }

    /// Returns the cached file content.
    pub fn content(&self) -> &[u8] {
        &self.content
    }
}
