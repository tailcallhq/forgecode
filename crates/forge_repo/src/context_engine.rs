use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use forge_app::GrpcInfra;
use forge_domain::{
    ApiKey, FileUploadInfo, Node, UserId, WorkspaceAuth, WorkspaceId, WorkspaceIndexRepository,
    WorkspaceInfo,
};

use crate::proto_generated::forge_service_client::ForgeServiceClient;
use crate::proto_generated::{self, *};

// TryFrom implementations for converting proto types to domain types

impl TryFrom<CreateApiKeyResponse> for WorkspaceAuth {
    type Error = anyhow::Error;

    fn try_from(response: CreateApiKeyResponse) -> Result<Self> {
        let user_id = response.user_id.context("Missing user_id in response")?.id;
        let user_id = UserId::from_string(&user_id).context("Invalid user_id returned from API")?;
        let token: ApiKey = response.key.into();

        Ok(WorkspaceAuth { user_id, token, created_at: Utc::now() })
    }
}

impl TryFrom<CreateWorkspaceResponse> for WorkspaceId {
    type Error = anyhow::Error;

    fn try_from(response: CreateWorkspaceResponse) -> Result<Self> {
        let workspace = response.workspace.context("No workspace in response")?;
        let workspace_id = workspace
            .workspace_id
            .context("Server did not return workspace ID in CreateWorkspace response")?
            .id;

        WorkspaceId::from_string(&workspace_id)
            .context("Failed to parse workspace ID from server response")
    }
}

impl TryFrom<Workspace> for WorkspaceInfo {
    type Error = anyhow::Error;

    fn try_from(workspace: Workspace) -> Result<Self> {
        let id_msg = workspace
            .workspace_id
            .context("Missing workspace_id in response")?;
        let workspace_id =
            WorkspaceId::from_string(&id_msg.id).context("Failed to parse workspace ID")?;

        let last_updated = workspace
            .last_updated
            .and_then(|ts| chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32));

        let created_at = workspace
            .created_at
            .and_then(|ts| chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32))
            .context("Missing or invalid created_at")?;

        Ok(WorkspaceInfo {
            workspace_id,
            working_dir: workspace.working_dir,
            node_count: workspace.node_count,
            relation_count: workspace.relation_count,
            last_updated,
            created_at,
        })
    }
}

impl TryFrom<FileRefNode> for forge_domain::FileHash {
    type Error = anyhow::Error;

    fn try_from(file_ref_node: FileRefNode) -> Result<Self> {
        let data = file_ref_node.data.context("Missing data in FileRefNode")?;
        Ok(forge_domain::FileHash { path: data.path, hash: data.file_hash })
    }
}

/// gRPC implementation of WorkspaceIndexRepository
///
/// This repository provides gRPC-based workspace operations.
pub struct ForgeContextEngineRepository<I> {
    infra: Arc<I>,
}

impl<I> ForgeContextEngineRepository<I> {
    /// Create a new repository with the given infrastructure
    ///
    /// # Arguments
    /// * `infra` - Infrastructure that provides gRPC connection
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }

    /// Add authorization header to a gRPC request
    ///
    /// Takes ownership of the request, adds the Bearer token to the
    /// authorization header, and returns the modified request.
    fn with_auth<T>(
        &self,
        mut request: tonic::Request<T>,
        auth_token: &ApiKey,
    ) -> Result<tonic::Request<T>> {
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", &**auth_token).parse()?,
        );
        Ok(request)
    }
}

#[async_trait]
impl<I: GrpcInfra> WorkspaceIndexRepository for ForgeContextEngineRepository<I> {
    async fn authenticate(&self) -> Result<WorkspaceAuth> {
        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        let request = tonic::Request::new(CreateApiKeyRequest { user_id: None });

        let response = client
            .create_api_key(request)
            .await
            .context("Failed to call CreateApiKey gRPC")?
            .into_inner();

        response.try_into()
    }

    async fn create_workspace(
        &self,
        working_dir: &std::path::Path,
        auth_token: &forge_domain::ApiKey,
    ) -> Result<WorkspaceId> {
        let request = tonic::Request::new(CreateWorkspaceRequest {
            workspace: Some(WorkspaceDefinition {
                working_dir: working_dir.to_string_lossy().replace("\\", "/"),
                ..Default::default()
            }),
        });

        let request = self.with_auth(request, auth_token)?;

        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        let response = client.create_workspace(request).await?.into_inner();

        response.try_into()
    }

    async fn upload_files(
        &self,
        upload: &forge_domain::FileUpload,
        auth_token: &forge_domain::ApiKey,
    ) -> Result<FileUploadInfo> {
        let files: Vec<File> = upload
            .data
            .iter()
            .map(|file_read| File {
                path: file_read.path.clone(),
                content: file_read.content.clone(),
            })
            .collect();

        let request = tonic::Request::new(UploadFilesRequest {
            workspace_id: Some(proto_generated::WorkspaceId {
                id: upload.workspace_id.to_string(),
            }),
            content: Some(FileUploadContent { files, git: None }),
        });

        let request = self.with_auth(request, auth_token)?;

        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        let response = client.upload_files(request).await?;

        let result = response
            .into_inner()
            .result
            .context("Server did not return upload result in UploadFiles response")?;

        Ok(FileUploadInfo::new(
            result.node_ids.len(),
            result.relations.len(),
        ))
    }

    /// Search for code using semantic search
    async fn search(
        &self,
        search_query: &forge_domain::CodeSearchQuery<'_>,
        auth_token: &forge_domain::ApiKey,
    ) -> Result<Vec<Node>> {
        let request = tonic::Request::new(SearchRequest {
            workspace_id: Some(proto_generated::WorkspaceId {
                id: search_query.workspace_id.to_string(),
            }),
            query: Some(Query {
                prompt: Some(search_query.data.query.to_string()),
                limit: search_query.data.limit.map(|l| l as u32),
                top_k: search_query.data.top_k,
                relevance_query: Some(search_query.data.use_case.to_string()),
                starts_with: search_query.data.starts_with.clone().into_iter().collect(),
                ends_with: search_query.data.ends_with.clone().unwrap_or_default(),
                max_distance: None,
                kinds: vec![NodeKind::FileChunk.into()],
            }),
        });

        let request = self.with_auth(request, auth_token)?;

        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        let response = client.search(request).await?;

        let result = response.into_inner().result.unwrap_or_default();

        // Convert QueryItems to CodeSearchResults
        let results = result
            .data
            .into_iter()
            .filter_map(|query_item| {
                let node = query_item.node?;
                let node_data = node.data?;
                let node_id = node.node_id.map(|n| n.id).unwrap_or_default();

                // Extract relevance and distance from proto (all optional)
                let relevance = query_item.relevance;
                let distance = query_item.distance;

                // Convert proto node to domain CodeNode based on type
                let code_node = match node_data.kind? {
                    node_data::Kind::FileChunk(chunk) => {
                        forge_domain::NodeData::FileChunk(forge_domain::FileChunk {
                            file_path: chunk.path,
                            content: chunk.content,
                            start_line: chunk.start_line,
                            end_line: chunk.end_line,
                        })
                    }
                    node_data::Kind::File(file) => {
                        forge_domain::NodeData::File(forge_domain::FileNode {
                            file_path: file.path,
                            content: file.content,
                            hash: node.hash,
                        })
                    }
                    node_data::Kind::FileRef(file_ref) => {
                        forge_domain::NodeData::FileRef(forge_domain::FileRef {
                            file_path: file_ref.path,
                            file_hash: file_ref.file_hash,
                        })
                    }
                    node_data::Kind::Note(note) => {
                        forge_domain::NodeData::Note(forge_domain::Note { content: note.content })
                    }
                    node_data::Kind::Task(task) => {
                        forge_domain::NodeData::Task(forge_domain::Task { task: task.task })
                    }
                };

                // Wrap the node with its relevance and distance scores
                Some(Node {
                    node_id: node_id.into(),
                    node: code_node,
                    relevance,
                    distance,
                })
            })
            .collect();

        Ok(results)
    }

    /// List all workspaces for a user
    async fn list_workspaces(
        &self,
        auth_token: &forge_domain::ApiKey,
    ) -> Result<Vec<WorkspaceInfo>> {
        let request = tonic::Request::new(ListWorkspacesRequest {});
        let request = self.with_auth(request, auth_token)?;

        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        let response = client.list_workspaces(request).await?;

        response
            .into_inner()
            .workspaces
            .into_iter()
            .map(|workspace| workspace.try_into())
            .collect()
    }

    /// Get workspace information by workspace ID
    async fn get_workspace(
        &self,
        workspace_id: &WorkspaceId,
        auth_token: &forge_domain::ApiKey,
    ) -> Result<Option<WorkspaceInfo>> {
        let request = tonic::Request::new(GetWorkspaceInfoRequest {
            workspace_id: Some(proto_generated::WorkspaceId { id: workspace_id.to_string() }),
        });
        let request = self.with_auth(request, auth_token)?;

        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        let response = client.get_workspace_info(request).await?;

        let workspace = response.into_inner().workspace;
        workspace.map(|w| w.try_into()).transpose()
    }

    /// List all files in a workspace with their hashes
    async fn list_workspace_files(
        &self,
        workspace: &forge_domain::WorkspaceFiles,
        auth_token: &forge_domain::ApiKey,
    ) -> Result<Vec<forge_domain::FileHash>> {
        let request = tonic::Request::new(ListFilesRequest {
            workspace_id: Some(proto_generated::WorkspaceId {
                id: workspace.workspace_id.to_string(),
            }),
        });

        let request = self.with_auth(request, auth_token)?;

        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        let response = client.list_files(request).await?;

        response
            .into_inner()
            .files
            .into_iter()
            .map(|file_ref_node| file_ref_node.try_into())
            .collect()
    }

    /// Delete files from a workspace
    async fn delete_files(
        &self,
        deletion: &forge_domain::FileDeletion,
        auth_token: &forge_domain::ApiKey,
    ) -> Result<()> {
        if deletion.data.is_empty() {
            return Ok(());
        }

        let request = tonic::Request::new(DeleteFilesRequest {
            workspace_id: Some(proto_generated::WorkspaceId {
                id: deletion.workspace_id.to_string(),
            }),
            file_paths: deletion.data.clone(),
        });

        let request = self.with_auth(request, auth_token)?;

        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        client.delete_files(request).await?;

        Ok(())
    }

    async fn delete_workspace(
        &self,
        workspace_id: &forge_domain::WorkspaceId,
        auth_token: &forge_domain::ApiKey,
    ) -> Result<()> {
        let request = tonic::Request::new(DeleteWorkspaceRequest {
            workspace_id: Some(proto_generated::WorkspaceId { id: workspace_id.to_string() }),
        });

        let request = self.with_auth(request, auth_token)?;

        let channel = self.infra.channel()?;
        let mut client = ForgeServiceClient::new(channel);
        client.delete_workspace(request).await?;

        Ok(())
    }
}
