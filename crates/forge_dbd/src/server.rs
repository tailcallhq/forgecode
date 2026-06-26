use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use crate::protocol::{read_frame, write_frame, Request, Response};

pub struct DbServer {
    socket_path: PathBuf,
    // TODO: Once diesel/forge_repo integration is ready, replace this with a real
    // DatabasePool connection. For now, we keep db_path as a marker.
    db_path: PathBuf,
    queue_tx: mpsc::Sender<QueuedRequest>,
}

struct QueuedRequest {
    request: Request,
    response_tx: tokio::sync::oneshot::Sender<Response>,
}

impl DbServer {
    pub fn new(socket_path: PathBuf, db_path: PathBuf) -> Self {
        let (queue_tx, _queue_rx) = mpsc::channel(1024);
        Self { socket_path, db_path, queue_tx }
    }

    pub async fn run(self) -> Result<()> {
        info!(socket = %self.socket_path.display(), db = %self.db_path.display(), "DbServer starting");

        // Remove stale socket if present
        if self.socket_path.exists() {
            warn!(path = %self.socket_path.display(), "removing stale socket");
            tokio::fs::remove_file(&self.socket_path).await?;
        }

        // Create parent directory if needed
        if let Some(parent) = self.socket_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Bind Unix domain socket
        let listener = UnixListener::bind(&self.socket_path)?;
        info!(socket = %self.socket_path.display(), "Unix socket bound");

        // Spawn the writer task that drains the queue and batches writes
        let (queue_tx, queue_rx) = mpsc::channel(1024);
        let _writer_handle = tokio::spawn(Self::writer_task(queue_rx));

        // Accept loop
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    debug!("client connected");
                    let queue_tx = queue_tx.clone();
                    tokio::spawn(Self::handle_client(stream, queue_tx));
                }
                Err(e) => {
                    error!("accept error: {e}");
                    // Continue accepting despite transient errors
                }
            }
        }
    }

    /// Handle a single client connection: read framed requests, enqueue them
    async fn handle_client(stream: UnixStream, queue_tx: mpsc::Sender<QueuedRequest>) {
        let stream = Arc::new(Mutex::new(stream));

        loop {
            // Read a single framed request
            let request = {
                let mut guard = stream.lock().await;
                match timeout(
                    Duration::from_secs(30),
                    read_frame::<_, Request>(&mut *guard),
                )
                .await
                {
                    Ok(Ok(req)) => req,
                    Ok(Err(e)) => {
                        debug!("frame read error: {e}");
                        break;
                    }
                    Err(_) => {
                        debug!("client read timeout");
                        break;
                    }
                }
            };

            debug!("received request: {:?}", request);

            // Create a oneshot for the response
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
            let queued = QueuedRequest { request, response_tx };

            // Enqueue the request
            if queue_tx.send(queued).await.is_err() {
                error!("failed to enqueue request; channel closed");
                let err_response = Response::Error {
                    message: "server queue closed".to_string(),
                };
                let mut guard = stream.lock().await;
                let _ = write_frame(&mut *guard, &err_response).await;
                break;
            }

            // Wait for response
            match timeout(Duration::from_secs(30), response_rx).await {
                Ok(Ok(response)) => {
                    debug!("sending response: {:?}", response);
                    let mut guard = stream.lock().await;
                    if let Err(e) = write_frame(&mut *guard, &response).await {
                        error!("failed to write response: {e}");
                        break;
                    }
                }
                Ok(Err(_)) => {
                    error!("response oneshot dropped");
                    break;
                }
                Err(_) => {
                    error!("response timeout");
                    let timeout_resp = Response::Error {
                        message: "server processing timeout".to_string(),
                    };
                    let mut guard = stream.lock().await;
                    let _ = write_frame(&mut *guard, &timeout_resp).await;
                    break;
                }
            }
        }

        debug!("client disconnected");
    }

    /// Writer task: batches requests arriving within a window, executes them in a transaction
    async fn writer_task(mut queue_rx: mpsc::Receiver<QueuedRequest>) {
        let mut batch: Vec<QueuedRequest> = Vec::new();
        let batch_timeout = Duration::from_millis(15);
        let batch_threshold = 100; // Flush when batch reaches this size

        loop {
            // Try to accumulate requests with a short timeout
            match timeout(batch_timeout, queue_rx.recv()).await {
                Ok(Some(req)) => {
                    batch.push(req);
                    if batch.len() >= batch_threshold {
                        Self::flush_batch(&mut batch).await;
                    }
                }
                Ok(None) => {
                    // Channel closed; flush remaining and exit
                    if !batch.is_empty() {
                        Self::flush_batch(&mut batch).await;
                    }
                    info!("writer task exiting");
                    break;
                }
                Err(_) => {
                    // Timeout: flush current batch
                    if !batch.is_empty() {
                        Self::flush_batch(&mut batch).await;
                    }
                }
            }
        }
    }

    /// Execute a batch of requests in a single transaction
    /// For now, send Ack for each; TODO: integrate with diesel/database
    async fn flush_batch(batch: &mut Vec<QueuedRequest>) {
        debug!(count = batch.len(), "flushing batch");

        // TODO: Open a single rusqlite/diesel transaction here, execute all
        // requests within it, and capture any error. For now, we acknowledge
        // each request successfully to verify the framing and queueing works.

        for queued in batch.drain(..) {
            let resp = Response::Ack; // TODO: real transaction execution
            let _ = queued.response_tx.send(resp);
        }
    }
}
