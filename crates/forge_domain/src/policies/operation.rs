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
    Execute { command: String, cwd: PathBuf },
    /// Network fetch operation with a URL
    Fetch {
        url: String,
        cwd: PathBuf,
        message: String,
    },
    /// MCP server connection authorization, identified by the server name as
    /// it appears in `.mcp.json`. Evaluated once per server when the MCP
    /// service brings up connections; the decision then gates every tool
    /// call routed through that server.
    Mcp { server: String, message: String },
}
