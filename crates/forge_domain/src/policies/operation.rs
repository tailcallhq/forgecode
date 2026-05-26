use std::path::PathBuf;

/// Operations that can be performed and need policy checking
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionOperation {
    /// Write operation to a file path
    Write {
        path: PathBuf,
        cwd: PathBuf,
        message: String,
    },
    /// Read operation from a file path
    Read {
        path: PathBuf,
        cwd: PathBuf,
        message: String,
    },
    /// Execute operation with a command string
    Execute {
        command: String,
        cwd: PathBuf,
        message: String,
    },
    /// Network fetch operation with a URL
    Fetch {
        url: String,
        cwd: PathBuf,
        message: String,
    },
}
