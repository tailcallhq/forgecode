use std::path::Path;

use anyhow::{Context, Result, bail};
use tokio::net::UnixStream;

use crate::protocol::{HealthStatus, Request, Response, read_frame, write_frame};

/// Client for the `forge_dbd` Unix-socket daemon.
///
/// Each call to [`DbClient::send`] opens a fresh connection so the client
/// remains simple and stateless. Connection pooling can be added later once
/// the protocol stabilises.
pub struct DbClient {
    socket_path: std::path::PathBuf,
}

impl DbClient {
    /// Create a client that will connect to the daemon at `socket_path`.
    ///
    /// This does **not** open a connection; use [`DbClient::send`] for that.
    pub async fn connect(socket_path: impl AsRef<Path>) -> Result<Self> {
        let socket_path = socket_path.as_ref().to_path_buf();
        // Verify the socket is reachable right away so callers get an early
        // error rather than failing on the first `send`.
        let _ = UnixStream::connect(&socket_path)
            .await
            .with_context(|| format!("cannot connect to forge_dbd at {}", socket_path.display()))?;
        Ok(Self { socket_path })
    }

    /// Send `request` to the daemon and return the response.
    pub async fn send(&self, request: Request) -> Result<Response> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .with_context(|| {
                format!(
                    "failed to connect to forge_dbd at {}",
                    self.socket_path.display()
                )
            })?;

        write_frame(&mut stream, &request)
            .await
            .context("failed to write request frame")?;

        let response: Response = read_frame(&mut stream)
            .await
            .context("failed to read response frame")?;

        Ok(response)
    }

    /// Query the daemon health status.
    ///
    /// Returns [`HealthStatus`] on success or an error if the daemon is
    /// unreachable or returns an unexpected response.
    pub async fn health(&self) -> Result<HealthStatus> {
        match self.send(Request::Ping).await? {
            Response::Health(s) => Ok(s),
            Response::Error { message } => bail!("daemon health error: {message}"),
            other => bail!("unexpected response to Ping: {other:?}"),
        }
    }
}
