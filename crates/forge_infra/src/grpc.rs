use std::sync::{Arc, Mutex};

use tonic::transport::Channel;
use url::Url;

/// Wrapper for a shared gRPC channel to the workspace server
///
/// This struct manages a lazily-connected gRPC channel that can be cheaply
/// cloned and shared across multiple gRPC clients. The channel is only created
/// on first access.
#[derive(Clone)]
pub struct ForgeGrpcClient {
    server_url: String,
    channel: Arc<Mutex<Option<Channel>>>,
}

impl ForgeGrpcClient {
    /// Creates a new gRPC client that will lazily connect on first use
    ///
    /// # Arguments
    /// * `server_url` - The URL of the gRPC server
    pub fn new(server_url: String) -> Self {
        Self { server_url, channel: Arc::new(Mutex::new(None)) }
    }

    /// Returns a clone of the underlying gRPC channel
    ///
    /// Channels are cheap to clone and can be shared across multiple clients.
    /// The channel is created on first call and cached for subsequent calls.
    pub fn channel(&self) -> anyhow::Result<Channel> {
        let mut guard = self.channel.lock().unwrap();

        if let Some(channel) = guard.as_ref() {
            return Ok(channel.clone());
        }

        let mut channel = Channel::from_shared(self.server_url.to_string())
            .expect("Invalid server URL")
            .concurrency_limit(256);

        // Enable TLS for https URLs (webpki-roots is faster than native-roots)
        if Url::parse(&self.server_url)?.scheme().contains("https") {
            let tls_config = tonic::transport::ClientTlsConfig::new().with_webpki_roots();
            channel = channel
                .tls_config(tls_config)
                .expect("Failed to configure TLS");
        }

        let new_channel = channel.connect_lazy();
        *guard = Some(new_channel.clone());
        Ok(new_channel)
    }

    /// Hydrates the gRPC channel by forcing its initialization
    ///
    /// This clears any existing cached channel and forces a fresh connection
    /// on the next call to `channel()`.
    /// Used to warm up or reset the connection.
    pub fn hydrate(&self) {
        let mut guard = self.channel.lock().unwrap();
        *guard = None;
    }
}
