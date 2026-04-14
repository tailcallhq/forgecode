use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use forge_app::GrpcInfra;
use forge_domain::{SyntaxError, ValidationRepository};
use tracing::{debug, warn};

use crate::proto_generated::forge_service_client::ForgeServiceClient;
use crate::proto_generated::{self, File, ValidateFilesRequest};

/// gRPC implementation of ValidationRepository
pub struct ForgeValidationRepository<I> {
    infra: Arc<I>,
}

impl<I> ForgeValidationRepository<I> {
    /// Create a new repository with the given infrastructure
    ///
    /// # Arguments
    /// * `infra` - Infrastructure that provides gRPC connection
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait]
impl<I: GrpcInfra> ValidationRepository for ForgeValidationRepository<I> {
    async fn validate_file(
        &self,
        path: impl AsRef<Path> + Send,
        content: &str,
    ) -> Result<Vec<SyntaxError>> {
        let path = path.as_ref();
        let path_str = path.to_string_lossy().to_string();

        debug!(path = %path_str, "Starting syntax validation");

        // Create validation request for single file
        let proto_file = File { path: path_str.clone(), content: content.to_string() };
        let request = tonic::Request::new(ValidateFilesRequest { files: vec![proto_file] });

        // Call gRPC API
        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        let response = client
            .validate_files(request)
            .await
            .context("Failed to call ValidateFiles gRPC")?
            .into_inner();

        // Extract validation result for our file
        let result = response
            .results
            .into_iter()
            .find(|r| r.file_path == path_str)
            .context("Validation response missing file result")?;

        // Convert proto status to error message
        match result.status {
            Some(proto_generated::ValidationStatus { status: Some(status) }) => match status {
                proto_generated::validation_status::Status::Valid(_) => {
                    debug!(path = %path_str, "Syntax validation passed");
                    Ok(vec![])
                }
                proto_generated::validation_status::Status::Errors(error_list) => {
                    if error_list.errors.is_empty() {
                        return Ok(vec![]);
                    }

                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("unknown");

                    let error_count = error_list.errors.len();

                    // Log and convert proto errors to domain errors
                    let errors = error_list
                        .errors
                        .into_iter()
                        .map(|error| {
                            warn!(
                                path = %path_str,
                                extension = ext,
                                error_count,
                                error_line = error.line,
                                error_column = error.column,
                                error_message = %error.message,
                                "Syntax validation failed"
                            );
                            SyntaxError {
                                line: error.line,
                                column: error.column,
                                message: error.message,
                            }
                        })
                        .collect();

                    Ok(errors)
                }
                proto_generated::validation_status::Status::UnsupportedLanguage(_) => {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("unknown");
                    debug!(
                        path = %path_str,
                        extension = ext,
                        "Syntax validation skipped: unsupported language"
                    );
                    Ok(vec![])
                }
            },
            _ => Ok(vec![]),
        }
    }
}
