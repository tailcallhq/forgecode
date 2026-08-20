// Public client for the forge_dbd daemon. Nothing outside this crate uses it
// yet (forge_app has not wired the daemon in), so it is all dead code until
// that integration lands.
#![allow(dead_code)]

use std::path::Path;

use anyhow::{Context, Result, bail};
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

#[cfg(windows)]
use crate::protocol::named_pipe_name;
use crate::protocol::{HealthStatus, Request, Response, read_frame, write_frame};

/// Client for the `forge_dbd` daemon.
///
/// Each call to [`DbClient::send`] opens a fresh connection so the client
/// remains simple and stateless. Connection pooling can be added later once
/// the protocol stabilises.
///
/// The transport is platform-dependent: a Unix domain socket on Unix, and a
/// Windows named pipe (derived deterministically from the socket path) on
/// Windows.
pub struct DbClient {
    socket_path: std::path::PathBuf,
}

impl DbClient {
    /// Create a client that will connect to the daemon at `socket_path`.
    ///
    /// This does **not** keep a connection open; use [`DbClient::send`] for
    /// that. The probe here just verifies the daemon is reachable so callers
    /// get an early error rather than failing on the first `send`.
    pub async fn connect(socket_path: impl AsRef<Path>) -> Result<Self> {
        let socket_path = socket_path.as_ref().to_path_buf();

        #[cfg(unix)]
        {
            let _ = UnixStream::connect(&socket_path).await.with_context(|| {
                format!("cannot connect to forge_dbd at {}", socket_path.display())
            })?;
        }

        #[cfg(windows)]
        {
            let pipe_name = named_pipe_name(&socket_path);
            let _ = Self::open_pipe(&pipe_name)
                .await
                .with_context(|| format!("cannot connect to forge_dbd at {pipe_name}"))?;
        }

        Ok(Self { socket_path })
    }

    /// Send `request` to the daemon and return the response.
    pub async fn send(&self, request: Request) -> Result<Response> {
        #[cfg(unix)]
        {
            let mut stream = UnixStream::connect(&self.socket_path)
                .await
                .with_context(|| {
                    format!(
                        "failed to connect to forge_dbd at {}",
                        self.socket_path.display()
                    )
                })?;
            Self::request_response(&mut stream, request).await
        }

        #[cfg(windows)]
        {
            let pipe_name = named_pipe_name(&self.socket_path);
            let mut stream = Self::open_pipe(&pipe_name)
                .await
                .with_context(|| format!("failed to connect to forge_dbd at {pipe_name}"))?;
            Self::request_response(&mut stream, request).await
        }
    }

    /// Opens the daemon pipe, retrying while the previous connection is still
    /// being torn down.
    ///
    /// The server accepts one client per pipe instance and only creates the
    /// next instance once the current connection drops, so there is a brief
    /// window where an instance exists but is not yet listening. CreateFileW
    /// reports that as `ERROR_PIPE_BUSY` (231); the canonical Windows client
    /// pattern is to wait briefly and retry.
    #[cfg(windows)]
    async fn open_pipe(pipe_name: &str) -> Result<NamedPipeClient> {
        for _ in 0..100 {
            match ClientOptions::new().open(pipe_name) {
                Ok(pipe) => return Ok(pipe),
                Err(e) if e.raw_os_error() == Some(231) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
        bail!("timed out waiting for forge_dbd pipe {pipe_name}")
    }

    /// Exchange one request/response frame pair over an open stream.
    ///
    /// The frame protocol is transport-agnostic, so both the unix socket and
    /// the named pipe reuse this body.
    async fn request_response<S>(stream: &mut S, request: Request) -> Result<Response>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        write_frame(stream, &request)
            .await
            .context("failed to write request frame")?;

        let response: Response = read_frame(stream)
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
