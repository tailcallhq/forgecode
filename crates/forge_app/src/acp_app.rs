use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use forge_config::ForgeConfig;

use crate::{EnvironmentInfra, Services};

/// ACP (Agent Communication Protocol) application orchestrator.
pub struct AcpApp<S> {
    services: Arc<S>,
}

/// Maximum time to wait for ACP I/O before considering the client hung.
const IO_TIMEOUT: Duration = Duration::from_secs(300);

/// Maximum time to wait for pending notifications to drain on shutdown.
const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

impl<S: Services + EnvironmentInfra<Config = ForgeConfig>> AcpApp<S> {
    /// Creates a new ACP application orchestrator.
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }

    /// Starts the ACP server over stdio transport.
    ///
    /// # Trust model
    ///
    /// The stdio transport inherits OS-level process isolation: only the
    /// parent process (e.g. Acepe) that spawned `forge machine stdio` can
    /// read/write the stdin/stdout pipes. No network listener is opened.
    /// Authentication is therefore a no-op by design.
    pub async fn start_stdio(&self) -> Result<()> {
        use agent_client_protocol as acp;
        use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

        let services = self.services.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to create Tokio runtime: {}", e))?;

            rt.block_on(async move {
                let (adapter, mut rx) = crate::acp::AcpAdapter::new(services);
                let adapter = Arc::new(adapter);

                let local_set = tokio::task::LocalSet::new();
                local_set
                    .run_until(async move {
                        let outgoing = tokio::io::stdout().compat_write();
                        let incoming = tokio::io::stdin().compat();

                        let (conn, handle_io) = acp::AgentSideConnection::new(
                            adapter.clone(),
                            outgoing,
                            incoming,
                            |fut| {
                                tokio::task::spawn_local(fut);
                            },
                        );

                        let conn = Arc::new(conn);
                        adapter.set_client_connection(conn.clone()).await;

                        let conn_for_notifications = conn.clone();
                        let notification_task = tokio::task::spawn_local(async move {
                            while let Some(session_notification) = rx.recv().await {
                                use agent_client_protocol::Client;

                                if let Err(error) = conn_for_notifications
                                    .session_notification(session_notification)
                                    .await
                                {
                                    tracing::error!(
                                        "Failed to send session notification: {}",
                                        error
                                    );
                                    break;
                                }
                            }
                        });

                        // Wait for I/O with a timeout to prevent indefinite hangs
                        // when the client stalls.
                        let io_result = match tokio::time::timeout(IO_TIMEOUT, handle_io).await {
                            Ok(result) => result,
                            Err(_) => {
                                tracing::warn!("ACP I/O timed out after {:?}", IO_TIMEOUT);
                                notification_task.abort();
                                return Err(anyhow::anyhow!(
                                    "ACP transport timed out after {:?}",
                                    IO_TIMEOUT
                                ));
                            }
                        };

                        // Graceful shutdown: give the notification task time to
                        // drain pending messages instead of aborting immediately.
                        drop(adapter); // drops the sender half → rx.recv() returns None
                        let _ = tokio::time::timeout(SHUTDOWN_DRAIN_TIMEOUT, notification_task).await;

                        io_result.map_err(|error| anyhow::anyhow!("ACP transport error: {}", error))
                    })
                    .await
            })
        });

        match handle.await {
            Ok(result) => result,
            Err(error) if error.is_cancelled() => {
                tracing::info!("ACP server task was cancelled");
                Ok(())
            }
            Err(error) => Err(anyhow::anyhow!("ACP server task panicked: {}", error)),
        }
    }
}
