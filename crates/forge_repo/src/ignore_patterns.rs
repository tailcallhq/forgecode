use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use forge_app::GrpcInfra;
use forge_domain::IgnorePatternsRepository;

use crate::proto_generated::ListIgnorePatternsRequest;
use crate::proto_generated::forge_service_client::ForgeServiceClient;

/// gRPC implementation of [`IgnorePatternsRepository`].
///
/// Fetches the raw server-side ignore patterns file so the CLI can apply the
/// same filtering rules as the server without re-implementing them locally.
pub struct ForgeIgnorePatternsRepository<I> {
    infra: Arc<I>,
}

impl<I> ForgeIgnorePatternsRepository<I> {
    /// Create a new repository backed by the provided gRPC infrastructure.
    ///
    /// # Arguments
    /// * `infra` - Infrastructure that provides the gRPC channel.
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait]
impl<I: GrpcInfra> IgnorePatternsRepository for ForgeIgnorePatternsRepository<I> {
    async fn list_ignore_patterns(&self) -> Result<String> {
        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        let response = client
            .list_ignore_patterns(tonic::Request::new(ListIgnorePatternsRequest {}))
            .await
            .context("Failed to call ListIgnorePatterns gRPC")?
            .into_inner();

        Ok(response.patterns)
    }
}
