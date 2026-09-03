use std::collections::BTreeMap;

use forge_app::McpServerInfra;
use forge_config::RetryConfig;
use forge_domain::{Environment, McpServerConfig};

use crate::mcp_client::ForgeMcpClient;

/// Constructs [`ForgeMcpClient`] instances, threading the global
/// [`RetryConfig`] so that retry/backoff, circuit-breaker thresholds, and the
/// bulkhead are driven by unified configuration rather than hard-coded values.
#[derive(Clone, Default)]
pub struct ForgeMcpServer {
    retry_config: RetryConfig,
}

impl ForgeMcpServer {
    pub fn new(retry_config: RetryConfig) -> Self {
        Self { retry_config }
    }
}

#[async_trait::async_trait]
impl McpServerInfra for ForgeMcpServer {
    type Client = ForgeMcpClient;

    async fn connect(
        &self,
        config: McpServerConfig,
        env_vars: &BTreeMap<String, String>,
        environment: &Environment,
    ) -> anyhow::Result<Self::Client> {
        Ok(ForgeMcpClient::with_retry_config(
            config,
            env_vars,
            environment.clone(),
            self.retry_config.clone(),
        ))
    }
}
