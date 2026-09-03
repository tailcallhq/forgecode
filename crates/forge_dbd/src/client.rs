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
use crate::protocol::{
    HealthStatus, MUTATION_PROTOCOL_VERSION, Request, Response, read_frame, write_frame,
};

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

/// The delivery certainty of a daemon request failure.
///
/// [`DbClientSendError::Unavailable`] means the fresh transport connection
/// could not be established, so no request bytes were sent and a caller may
/// safely choose its direct-storage fallback.
/// [`DbClientSendError::Indeterminate`] means the connection was established
/// and the request exchange began; the daemon may have observed the request, so
/// callers must not replay it.
#[derive(Debug)]
pub enum DbClientSendError {
    Unavailable(anyhow::Error),
    Indeterminate(anyhow::Error),
}

impl DbClientSendError {
    /// Discard delivery certainty while preserving the underlying failure for
    /// callers that retain the legacy [`DbClient::send`] API.
    fn into_anyhow(self) -> anyhow::Error {
        match self {
            Self::Unavailable(error) | Self::Indeterminate(error) => error,
        }
    }
}

impl std::fmt::Display for DbClientSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(error) => {
                write!(formatter, "daemon unavailable before send: {error}")
            }
            Self::Indeterminate(error) => {
                write!(
                    formatter,
                    "daemon request delivery is indeterminate: {error}"
                )
            }
        }
    }
}

impl std::error::Error for DbClientSendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable(error) | Self::Indeterminate(error) => Some(error.as_ref()),
        }
    }
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
            let mut stream = UnixStream::connect(&socket_path).await.with_context(|| {
                format!("cannot connect to forge_dbd at {}", socket_path.display())
            })?;
            Self::verify_mutation_protocol(&mut stream).await?;
        }

        #[cfg(windows)]
        {
            let pipe_name = named_pipe_name(&socket_path);
            let mut stream = Self::open_pipe(&pipe_name)
                .await
                .with_context(|| format!("cannot connect to forge_dbd at {pipe_name}"))?;
            Self::verify_mutation_protocol(&mut stream).await?;
        }

        Ok(Self { socket_path })
    }

    /// Verify that a daemon supports the scoped mutation envelope before a
    /// caller sends any database-changing request.
    async fn verify_mutation_protocol<S>(stream: &mut S) -> Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        match Self::request_response(stream, Request::Ping).await? {
            Response::Health(status) if status.protocol_version >= MUTATION_PROTOCOL_VERSION => {
                Ok(())
            }
            Response::Health(status) => bail!(
                "forge_dbd protocol v{} is too old for scoped mutations (requires v{})",
                status.protocol_version,
                MUTATION_PROTOCOL_VERSION,
            ),
            Response::Error { message } => bail!("forge_dbd protocol probe failed: {message}"),
            other => bail!("unexpected response to protocol probe: {other:?}"),
        }
    }

    /// Send `request` to the daemon and return the response.
    ///
    /// This compatibility API erases delivery certainty. Call
    /// [`DbClient::send_classified`] when a caller needs to decide whether a
    /// direct fallback can safely replay the request.
    pub async fn send(&self, request: Request) -> Result<Response> {
        self.send_classified(request)
            .await
            .map_err(DbClientSendError::into_anyhow)
    }

    /// Send `request` while preserving whether it was safe to replay.
    ///
    /// Only a fresh transport connection failure is
    /// [`DbClientSendError::Unavailable`]. Once a stream has connected,
    /// write, read, framing, decode, and response failures are all
    /// [`DbClientSendError::Indeterminate`].
    pub async fn send_classified(
        &self,
        request: Request,
    ) -> std::result::Result<Response, DbClientSendError> {
        #[cfg(unix)]
        {
            let mut stream = UnixStream::connect(&self.socket_path)
                .await
                .with_context(|| {
                    format!(
                        "failed to connect to forge_dbd at {}",
                        self.socket_path.display()
                    )
                })
                .map_err(DbClientSendError::Unavailable)?;
            Self::request_response(&mut stream, request)
                .await
                .map_err(DbClientSendError::Indeterminate)
        }

        #[cfg(windows)]
        {
            let pipe_name = named_pipe_name(&self.socket_path);
            let mut stream = Self::open_pipe(&pipe_name)
                .await
                .with_context(|| format!("failed to connect to forge_dbd at {pipe_name}"))
                .map_err(DbClientSendError::Unavailable)?;
            Self::request_response(&mut stream, request)
                .await
                .map_err(DbClientSendError::Indeterminate)
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

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[cfg(unix)]
    use super::{DbClient, DbClientSendError};
    #[cfg(unix)]
    use crate::protocol::Request;

    #[cfg(unix)]
    fn test_socket_path(label: &str) -> std::path::PathBuf {
        // macOS Unix sockets have a short SUN_LEN limit; its per-user temp
        // directory is often already longer than that limit.
        std::path::PathBuf::from(format!(
            "/tmp/fdb-{label}-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time is after Unix epoch")
                .as_nanos()
        ))
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn classifies_fresh_socket_connection_failure_as_unavailable() {
        let socket_path = test_socket_path("unavailable");
        // `connect` is deliberately eager, so construct the otherwise
        // ordinary client to exercise the per-request fresh connection path.
        let client = DbClient { socket_path };
        let error = client
            .send_classified(Request::Ping)
            .await
            .expect_err("a fresh nonexistent socket must be unavailable");

        assert!(matches!(error, DbClientSendError::Unavailable(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn classifies_failure_after_connection_as_indeterminate() {
        let socket_path = test_socket_path("indeterminate");
        let listener =
            tokio::net::UnixListener::bind(&socket_path).expect("test socket listener binds");
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("client connects");
            // Drop the accepted stream without replying. The connection was
            // established, but the caller cannot know whether a request was
            // observed, so it must not use a replay-safe classification.
        });

        let client = DbClient { socket_path: socket_path.clone() };
        let error = client
            .send_classified(Request::Ping)
            .await
            .expect_err("a peer that closes without a response must fail");
        server.await.expect("test server completes");
        std::fs::remove_file(socket_path).expect("test socket is removable");

        assert!(matches!(error, DbClientSendError::Indeterminate(_)));
    }

    /// A client must refuse an older daemon before it sends a mutation. Older
    /// daemons report a Health frame without the protocol-version field, so
    /// the caller can safely choose its direct-storage fallback.
    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_legacy_daemon_during_non_mutating_protocol_probe() {
        let socket_path = test_socket_path("legacy-protocol");
        let listener =
            tokio::net::UnixListener::bind(&socket_path).expect("test socket listener binds");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("client connects");

            let mut len = [0u8; 4];
            stream
                .read_exact(&mut len)
                .await
                .expect("read probe length");
            let mut request = vec![0; u32::from_le_bytes(len) as usize];
            stream
                .read_exact(&mut request)
                .await
                .expect("read probe request");
            assert!(
                request
                    .windows(b"Ping".len())
                    .any(|window| window == b"Ping")
            );

            // Exact v1 Health JSON: no protocol_version field.
            let response =
                br#"{\"Health\":{\"uptime_secs\":0,\"queue_depth\":0,\"db_reachable\":true}}"#;
            stream
                .write_all(&(response.len() as u32).to_le_bytes())
                .await
                .expect("write legacy response length");
            stream
                .write_all(response)
                .await
                .expect("write legacy response");
        });

        let actual = DbClient::connect(&socket_path).await;
        server.await.expect("legacy server completes");
        std::fs::remove_file(socket_path).expect("test socket is removable");

        assert!(
            actual.is_err(),
            "legacy protocol must be rejected before mutation"
        );
    }
}
