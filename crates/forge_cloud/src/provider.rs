//! Cloud provider trait and implementations.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::task::CloudTask;

// ---------------------------------------------------------------------------
// CloudProvider trait
// ---------------------------------------------------------------------------

/// Abstraction over cloud compute backends.
///
/// Implementors expose a minimal RPC surface: dispatch a [`CloudTask`], poll
/// its status, cancel it, and retrieve the final result.
#[async_trait]
pub trait CloudProvider: Send + Sync {
    /// Human-readable provider name (e.g. `"cloudflare-workers"`).
    fn name(&self) -> &'static str;

    /// Dispatch a task and return its id.
    async fn dispatch(&self, task: CloudTask) -> Result<Uuid>;

    /// Poll the current status of a task.
    async fn status(&self, task_id: Uuid) -> Result<TaskStatusResponse>;

    /// Attempt to cancel a running / queued task.
    async fn cancel(&self, task_id: Uuid) -> Result<()>;

    /// Retrieve the final result once the task has completed.
    async fn result(&self, task_id: Uuid) -> Result<CloudTaskResult>;

    /// Health check — returns `true` if the backend is reachable.
    async fn health(&self) -> bool;
}

// ---------------------------------------------------------------------------
// Shared response types
// ---------------------------------------------------------------------------

/// Response returned when querying task status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatusResponse {
    pub task_id: Uuid,
    pub state: TaskState,
    /// Optional progress message from the worker.
    pub message: Option<String>,
}

/// High-level task state (used for provider responses; mirrors
/// [`crate::task::TaskStatus`] but owned here so providers don't leak the full
/// task model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// The final result payload of a completed cloud task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudTaskResult {
    pub task_id: Uuid,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// Cloudflare Workers provider
// ---------------------------------------------------------------------------

/// Dispatches tasks to [Cloudflare Workers](https://developers.cloudflare.com/workers/).
///
/// Internally this talks to the Cloudflare API using an HTTP client
/// (reqwest). The worker is expected to accept a POST of [`CloudTask`] JSON
/// and respond with a task ID that can be polled.
pub struct CloudflareWorkers {
    api_token: String,
    account_id: String,
    /// Base URL for the Workers API.
    api_base: String,
    /// Optional endpoint for a specific worker.
    worker_name: Option<String>,
    client: reqwest::Client,
}

impl CloudflareWorkers {
    /// Create a new provider from explicit credentials.
    pub fn new(api_token: String, account_id: String) -> Self {
        let api_base = format!("https://api.cloudflare.com/client/v4/accounts/{account_id}");
        Self {
            api_token,
            account_id,
            api_base,
            worker_name: None,
            client: reqwest::Client::new(),
        }
    }

    /// Set a specific worker name to dispatch to.
    pub fn with_worker(mut self, worker_name: String) -> Self {
        self.worker_name = Some(worker_name);
        self
    }

    /// Override the API base URL (useful for testing).
    pub fn with_api_base(mut self, base: String) -> Self {
        self.api_base = base;
        self
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_token)
    }
}

#[async_trait]
impl CloudProvider for CloudflareWorkers {
    fn name(&self) -> &'static str {
        "cloudflare-workers"
    }

    async fn dispatch(&self, task: CloudTask) -> Result<Uuid> {
        let url = match &self.worker_name {
            Some(name) => {
                // Route through the specific worker's dispatch endpoint.
                format!("https://{name}.{}.workers.dev/dispatch", self.account_id)
            }
            None => {
                // Generic dispatch — account-level.
                format!("{}/dispatch", self.api_base)
            }
        };

        let task_id = task.id;
        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&task)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Cloudflare Workers dispatch failed (HTTP {status}): {body}");
        }

        tracing::info!(%task_id, "dispatched to Cloudflare Workers");
        Ok(task_id)
    }

    async fn status(&self, task_id: Uuid) -> Result<TaskStatusResponse> {
        let url = match &self.worker_name {
            Some(name) => format!(
                "https://{name}.{}.workers.dev/tasks/{task_id}",
                self.account_id
            ),
            None => format!("{}/tasks/{task_id}", self.api_base),
        };

        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("status query failed (HTTP {status}): {body}");
        }

        let status_resp: TaskStatusResponse = resp.json().await?;
        Ok(status_resp)
    }

    async fn cancel(&self, task_id: Uuid) -> Result<()> {
        let url = match &self.worker_name {
            Some(name) => format!(
                "https://{name}.{}.workers.dev/tasks/{task_id}/cancel",
                self.account_id
            ),
            None => format!("{}/tasks/{task_id}/cancel", self.api_base),
        };

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("cancel failed (HTTP {status}): {body}");
        }

        tracing::info!(%task_id, "task cancelled");
        Ok(())
    }

    async fn result(&self, task_id: Uuid) -> Result<CloudTaskResult> {
        let url = match &self.worker_name {
            Some(name) => format!(
                "https://{name}.{}.workers.dev/tasks/{task_id}/result",
                self.account_id
            ),
            None => format!("{}/tasks/{task_id}/result", self.api_base),
        };

        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("result fetch failed (HTTP {status}): {body}");
        }

        let result: CloudTaskResult = resp.json().await?;
        Ok(result)
    }

    async fn health(&self) -> bool {
        let url = match &self.worker_name {
            Some(name) => {
                format!("https://{name}.{}.workers.dev/health", self.account_id)
            }
            None => format!("{}/health", self.api_base),
        };

        self.client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Local fallback provider
// ---------------------------------------------------------------------------

/// A local, in-process provider used for development and offline operation.
///
/// Tasks are executed sequentially inside a background tokio task. State is
/// kept in a [`tokio::sync::RwLock`] so the runner can poll it.
pub struct LocalProvider {
    /// In-memory task store keyed by task ID.
    tasks: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<Uuid, LocalTaskEntry>>>,
}

#[derive(Debug)]
struct LocalTaskEntry {
    task: CloudTask,
    state: TaskState,
    result: Option<CloudTaskResult>,
}

impl Default for LocalProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalProvider {
    pub fn new() -> Self {
        Self {
            tasks: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }
}

#[async_trait]
impl CloudProvider for LocalProvider {
    fn name(&self) -> &'static str {
        "local"
    }

    async fn dispatch(&self, task: CloudTask) -> Result<Uuid> {
        let task_id = task.id;

        // Store as queued.
        {
            let mut map = self.tasks.write().await;
            map.insert(
                task_id,
                LocalTaskEntry { task, state: TaskState::Queued, result: None },
            );
        }

        // Spawn a simulated execution.
        let tasks = std::sync::Arc::clone(&self.tasks);
        tokio::spawn(async move {
            // Transition to Running.
            {
                let mut map = tasks.write().await;
                if let Some(entry) = map.get_mut(&task_id) {
                    entry.state = TaskState::Running;
                }
            }

            // Simulate work (a short sleep).
            let start = std::time::Instant::now();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let duration_ms = start.elapsed().as_millis() as u64;

            // Transition to Completed.
            {
                let mut map = tasks.write().await;
                if let Some(entry) = map.get_mut(&task_id) {
                    entry.state = TaskState::Completed;
                    entry.result = Some(CloudTaskResult {
                        task_id,
                        success: true,
                        output: Some(format!("local: task {} completed", entry.task.kind)),
                        error: None,
                        duration_ms: Some(duration_ms),
                    });
                }
            }
        });

        tracing::debug!(%task_id, "dispatched locally (in-process)");
        Ok(task_id)
    }

    async fn status(&self, task_id: Uuid) -> Result<TaskStatusResponse> {
        let map = self.tasks.read().await;
        let entry = map
            .get(&task_id)
            .ok_or_else(|| anyhow::anyhow!("task {task_id} not found"))?;

        Ok(TaskStatusResponse { task_id, state: entry.state, message: None })
    }

    async fn cancel(&self, task_id: Uuid) -> Result<()> {
        let mut map = self.tasks.write().await;
        let entry = map
            .get_mut(&task_id)
            .ok_or_else(|| anyhow::anyhow!("task {task_id} not found"))?;

        match entry.state {
            TaskState::Queued | TaskState::Running => {
                entry.state = TaskState::Cancelled;
                tracing::info!(%task_id, "cancelled local task");
                Ok(())
            }
            other => Err(anyhow::anyhow!("cannot cancel task in state {other:?}")),
        }
    }

    async fn result(&self, task_id: Uuid) -> Result<CloudTaskResult> {
        let map = self.tasks.read().await;
        let entry = map
            .get(&task_id)
            .ok_or_else(|| anyhow::anyhow!("task {task_id} not found"))?;

        entry
            .result
            .clone()
            .ok_or_else(|| anyhow::anyhow!("task {task_id} has not finished"))
    }

    async fn health(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{CloudTask, TaskPriority};

    fn sample_task() -> CloudTask {
        CloudTask::new("agent_run".into(), "summarize the codebase".into())
            .with_priority(TaskPriority::Normal)
    }

    #[tokio::test]
    async fn local_provider_dispatch_and_status() {
        let provider = LocalProvider::new();
        let task = sample_task();
        let id = provider.dispatch(task).await.unwrap();

        // Give the spawned task time to complete.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let status = provider.status(id).await.unwrap();
        assert_eq!(status.state, TaskState::Completed);
    }

    #[tokio::test]
    async fn local_provider_cancel() {
        let provider = LocalProvider::new();
        let task = sample_task();
        let id = provider.dispatch(task).await.unwrap();

        // Cancel immediately (might be queued).
        let _ = provider.cancel(id).await;

        let status = provider.status(id).await.unwrap();
        assert_eq!(status.state, TaskState::Cancelled);
    }

    #[tokio::test]
    async fn local_provider_health() {
        let provider = LocalProvider::new();
        assert!(provider.health().await);
    }
}
