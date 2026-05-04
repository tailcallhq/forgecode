use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::server::RpcModule;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// STDIO transport for JSON-RPC.
///
/// Reads JSON-RPC request objects (one per line) from stdin, dispatches them
/// to the registered `RpcModule`, and writes responses plus subscription
/// notifications (one JSON object per line) to stdout.
pub struct StdioTransport {
    module: RpcModule<()>,
}

impl StdioTransport {
    pub fn new(module: RpcModule<()>) -> Self {
        Self { module }
    }

    /// Run the transport loop over real stdin/stdout until stdin is closed.
    pub async fn run(self) -> anyhow::Result<()> {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        self.run_with_io(stdin, stdout).await
    }

    /// Run the transport loop over the given reader/writer until the reader
    /// reaches EOF.
    ///
    /// This is the core implementation used by [`run`] and available for
    /// testing with synthetic I/O streams.
    async fn run_with_io<R, W>(self, reader: R, writer: W) -> anyhow::Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let reader = BufReader::new(reader);
        let mut lines = reader.lines();
        let writer = Arc::new(Mutex::new(writer));

        let mut handles = Vec::new();

        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed: String = line.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }

            // Parse JSON to detect if this is a notification (no id).
            let request: Value = match serde_json::from_str(&trimmed) {
                Ok(req) => req,
                Err(e) => {
                    let error_response = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {
                            "code": -32700,
                            "message": format!("Parse error: {e}")
                        }
                    });
                    Self::write_line(&writer, &error_response).await?;
                    continue;
                }
            };

            // For notifications (no id), we still process but don't send a
            // response.
            let is_notification = request.get("id").is_none();

            let module = self.module.clone();
            let writer_clone = Arc::clone(&writer);

            let handle = tokio::spawn(async move {
                match module.raw_json_request(&trimmed, 1024 * 1024).await {
                    Ok((response_json, mut rx)) => {
                        // Send the initial response (or subscription
                        // acceptance).
                        if !is_notification {
                            if let Ok(response) =
                                serde_json::from_str::<Value>(&response_json)
                            {
                                let _ = Self::write_line(&writer_clone, &response).await;
                            }
                        }

                        // Forward subscription notifications.
                        // rx is a tokio::sync::mpsc::Receiver<String>
                        loop {
                            match rx.recv().await {
                                Some(notification) => {
                                    if let Ok(notif_value) =
                                        serde_json::from_str::<Value>(&notification)
                                    {
                                        if Self::write_line(&writer_clone, &notif_value)
                                            .await
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                }
                                None => break,
                            }
                        }
                    }
                    Err(e) => {
                        if !is_notification {
                            let error_response = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": null,
                                "error": {
                                    "code": -32603,
                                    "message": format!("Internal error: {e}")
                                }
                            });
                            let _ = Self::write_line(&writer_clone, &error_response).await;
                        }
                    }
                }
            });
            handles.push(handle);
        }

        // Wait for all dispatched requests to complete before returning.
        // Without this, the process exits before async work finishes when
        // stdin is a pipe (stdin EOF is reached immediately after the last
        // line is read).
        //
        // Use a per-handle timeout to avoid hanging indefinitely on
        // long-running subscriptions.
        for handle in handles {
            match tokio::time::timeout(Duration::from_secs(60), handle).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::warn!("JSON-RPC task failed: {e}");
                }
                Err(_) => {
                    tracing::warn!("JSON-RPC task timed out after 60s");
                }
            }
        }

        Ok(())
    }

    /// Write a single JSON line to writer, flushing afterwards.
    async fn write_line<W: AsyncWrite + Unpin>(
        writer: &Arc<Mutex<W>>,
        value: &Value,
    ) -> anyhow::Result<()> {
        let json = serde_json::to_string(value)?;
        let mut guard: tokio::sync::MutexGuard<'_, W> = writer.lock().await;
        guard.write_all(json.as_bytes()).await?;
        guard.write_all(b"\n").await?;
        guard.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use jsonrpsee::RpcModule;
    use serde_json::{json, Value};
    use tokio::io::AsyncReadExt;

    use super::*;

    #[tokio::test]
    async fn test_transport_returns_response_for_valid_request() {
        // Given a module with a simple echo method and a transport wrapping it
        let mut module = RpcModule::new(());
        module
            .register_async_method("echo", |params, _, _| {
                let params: Value = params.parse().unwrap_or(Value::Null);
                async move { Ok::<_, jsonrpsee::types::ErrorObjectOwned>(params) }
            })
            .expect("register echo");

        let transport = StdioTransport::new(module);

        // When we pipe a JSON-RPC request through synthetic I/O
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"echo","params":"hello"}"#;

        let (reader, mut writer) = tokio::io::duplex(4096);
        let (mut stdout_reader, stdout_writer) = tokio::io::duplex(4096);

        // Write the request to the reader side
        writer.write_all(request.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        // Drop writer so the transport sees EOF after the line
        drop(writer);

        // Run the transport (this will read the line, process it, and return)
        transport
            .run_with_io(reader, stdout_writer)
            .await
            .expect("transport should complete");

        // Then the response should be available on stdout
        let mut output = String::new();
        stdout_reader
            .read_to_string(&mut output)
            .await
            .unwrap();
        let response: Value = serde_json::from_str(output.trim()).unwrap();

        let expected = json!({"jsonrpc":"2.0","id":1,"result":"hello"});
        assert_eq!(response, expected);
    }

    #[tokio::test]
    async fn test_transport_returns_error_for_invalid_json() {
        // Given a transport wrapping an empty module
        let module = RpcModule::new(());
        let transport = StdioTransport::new(module);

        // When invalid JSON is sent
        let (reader, mut writer) = tokio::io::duplex(4096);
        let (mut stdout_reader, stdout_writer) = tokio::io::duplex(4096);

        writer.write_all(b"not valid json\n").await.unwrap();
        drop(writer);

        transport
            .run_with_io(reader, stdout_writer)
            .await
            .expect("transport should complete");

        let mut output = String::new();
        stdout_reader
            .read_to_string(&mut output)
            .await
            .unwrap();
        let response: Value = serde_json::from_str(output.trim()).unwrap();

        // Should be a parse error response
        assert_eq!(response["id"], json!(null));
        assert_eq!(response["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn test_transport_handles_unknown_method() {
        // Given a transport with an empty module (no methods registered)
        let module = RpcModule::new(());
        let transport = StdioTransport::new(module);

        // When a request for an unknown method is sent
        let request = r#"{"jsonrpc":"2.0","id":42,"method":"unknown","params":[]}"#;

        let (reader, mut writer) = tokio::io::duplex(4096);
        let (mut stdout_reader, stdout_writer) = tokio::io::duplex(4096);

        writer.write_all(request.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        drop(writer);

        transport
            .run_with_io(reader, stdout_writer)
            .await
            .expect("transport should complete");

        let mut output = String::new();
        stdout_reader
            .read_to_string(&mut output)
            .await
            .unwrap();
        let response: Value = serde_json::from_str(output.trim()).unwrap();

        // Should be an internal error response (method not found from
        // jsonrpsee)
        assert_eq!(response["id"], 42);
        assert!(response["error"].is_object());
    }
}
