use std::path::PathBuf;

use crate::mcp::McpServerConfig;

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
    Execute { command: String, cwd: PathBuf },
    /// Network fetch operation with a URL
    Fetch {
        url: String,
        cwd: PathBuf,
        message: String,
    },
    /// MCP server connection authorization. Evaluated once per server when the
    /// MCP service brings up connections; the decision then gates every tool
    /// call routed through that server. The `config` field carries either a
    /// stdio server (command + args) or an HTTP server (url) — never both.
    Mcp {
        /// The server configuration — either `Stdio` (command + args) or `Http` (url).
        config: McpServerConfig,
        /// The current working directory at the time of the operation.
        cwd: PathBuf,
        message: String,
    },
}
