//! High-level task runner that wraps a [`CloudProvider`] and manages
//! dispatch, polling, cancellation, and timeouts.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::provider::{CloudProvider, TaskState};
use crate::task::{CloudTask, TaskResult, TaskStatus};

// ---------------------------------------------------------------------------
// CloudRunner
// ---------------------------------------------------------------------------

/// Orchestrates agent tasks through a cloud backend.
///
/// The runner is cheaply cloneable — internal state is behind an `Arc`.
#[derive(Clone)]
pub struct CloudRunner {
    provider: Arc<dyn CloudProvider>,
    /// Active handles (task ID → last known status).
    handles: Arc<RwLock<HashMap<Uuid, TaskHandle>>>,
    /// Default timeout applied when a task doesn't specify one.
    default_timeout: std::time::Duration,
}

/// Lightweight handle returned on dispatch for the caller to poll / cancel.
#[derive(Debug, Clone)]
pub struct TaskHandle {
    pub task_id: Uuid,
    pub status: TaskStatus,
}

impl CloudRunner {
    /// Create a new runner backed by the given provider.
    pub fn new(provider: Arc<dyn CloudProvider>) -> Self {
        Self {
            provider,
            handles: Arc::new(RwLock::new(HashMap::new())),
            default_timeout: std::time::Duration::from_secs(300),
        }
    }

    /// Override the default task timeout.
    pub fn with_default_timeout(mut self, d: std::time::Duration) -> Self {
        self.default_timeout = d;
        self
    }

    /// Dispatch a task, returning a [`TaskHandle`] the caller can use to
    /// track progress.
    pub async fn dispatch(&self, task: CloudTask) -> Result<TaskHandle> {
        let id = self.provider.dispatch(task).await?;

        let handle = TaskHandle { task_id: id, status: TaskStatus::Queued };

        self.handles.write().await.insert(id, handle.clone());
        tracing::info!(%id, "task dispatched");

        Ok(handle)
    }

    /// Dispatch a task and block until it reaches a terminal state or the
    /// timeout expires.
    pub async fn dispatch_and_wait(&self, task: CloudTask) -> Result<TaskResult> {
        let timeout = task.timeout.unwrap_or(self.default_timeout);

        let handle = self.dispatch(task).await?;

        tokio::time::timeout(timeout, self.wait_for_terminal(handle.task_id))
            .await
            .map_err(|_| anyhow::anyhow!("task {} timed out after {:?}", handle.task_id, timeout))?
    }

    /// Poll until the task reaches a terminal state.
    async fn wait_for_terminal(&self, task_id: Uuid) -> Result<TaskResult> {
        let poll_interval = std::time::Duration::from_millis(250);
        loop {
            let status_resp = self.provider.status(task_id).await?;
            let mapped = map_provider_state(status_resp.state);

            // Update local handle.
            {
                let mut handles = self.handles.write().await;
                if let Some(h) = handles.get_mut(&task_id) {
                    h.status = mapped;
                }
            }

            if mapped.is_terminal() {
                // Fetch the full result.
                let result = self.provider.result(task_id).await?;
                return Ok(TaskResult {
                    task_id,
                    status: mapped,
                    output: result.output,
                    error: result.error.map(|e| crate::task::TaskError {
                        code: "PROVIDER_ERROR".into(),
                        message: e,
                        stack_trace: None,
                    }),
                    duration_ms: result.duration_ms,
                    finished_at: chrono::Utc::now(),
                });
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Poll the status of a task.
    pub async fn status(&self, task_id: Uuid) -> Result<TaskStatus> {
        let resp = self.provider.status(task_id).await?;
        Ok(map_provider_state(resp.state))
    }

    /// Cancel a running / queued task.
    pub async fn cancel(&self, task_id: Uuid) -> Result<()> {
        self.provider.cancel(task_id).await?;

        let mut handles = self.handles.write().await;
        if let Some(h) = handles.get_mut(&task_id) {
            h.status = TaskStatus::Cancelled;
        }

        tracing::info!(%task_id, "task cancelled via runner");
        Ok(())
    }

    /// Cancel all active tasks.
    pub async fn cancel_all(&self) -> Result<()> {
        let ids: Vec<Uuid> = {
            let handles = self.handles.read().await;
            handles
                .values()
                .filter(|h| !h.status.is_terminal())
                .map(|h| h.task_id)
                .collect()
        };

        for id in ids {
            if let Err(e) = self.cancel(id).await {
                tracing::warn!(%id, ?e, "failed to cancel task");
            }
        }

        Ok(())
    }

    /// Retrieve the full result for a completed task.
    pub async fn result(&self, task_id: Uuid) -> Result<TaskResult> {
        let resp = self.provider.result(task_id).await?;
        Ok(TaskResult {
            task_id,
            status: TaskStatus::Completed,
            output: resp.output,
            error: resp.error.map(|e| crate::task::TaskError {
                code: "PROVIDER_ERROR".into(),
                message: e,
                stack_trace: None,
            }),
            duration_ms: resp.duration_ms,
            finished_at: chrono::Utc::now(),
        })
    }

    /// Health check on the underlying provider.
    pub async fn health(&self) -> bool {
        self.provider.health().await
    }

    /// Number of tracked (non-terminal) tasks.
    pub async fn active_count(&self) -> usize {
        let handles = self.handles.read().await;
        handles.values().filter(|h| !h.status.is_terminal()).count()
    }
}

/// Map a provider-specific [`TaskState`] to the runner-level [`TaskStatus`].
fn map_provider_state(state: TaskState) -> TaskStatus {
    match state {
        TaskState::Queued => TaskStatus::Queued,
        TaskState::Running => TaskStatus::Running,
        TaskState::Completed => TaskStatus::Completed,
        TaskState::Failed => TaskStatus::Failed,
        TaskState::Cancelled => TaskStatus::Cancelled,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::LocalProvider;
    use crate::task::TaskPriority;

    fn local_runner() -> CloudRunner {
        CloudRunner::new(Arc::new(LocalProvider::new()))
    }

    #[tokio::test]
    async fn dispatch_and_status() {
        let runner = local_runner();
        let task =
            CloudTask::new("echo".into(), "hello".into()).with_priority(TaskPriority::Normal);

        let handle = runner.dispatch(task).await.unwrap();
        assert_eq!(handle.status, TaskStatus::Queued);

        // Wait a bit for the local provider to process.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let status = runner.status(handle.task_id).await.unwrap();
        assert!(status.is_terminal());
    }

    #[tokio::test]
    async fn cancel_task() {
        let runner = local_runner();
        let task = CloudTask::new("long".into(), "work".into());

        let handle = runner.dispatch(task).await.unwrap();
        let _ = runner.cancel(handle.task_id).await;

        let status = runner.status(handle.task_id).await.unwrap();
        assert_eq!(status, TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn cancel_all() {
        let runner = local_runner();
        let t1 = CloudTask::new("a".into(), "1".into());
        let t2 = CloudTask::new("b".into(), "2".into());

        let h1 = runner.dispatch(t1).await.unwrap();
        let _h2 = runner.dispatch(t2).await.unwrap();

        runner.cancel_all().await.unwrap();

        let s1 = runner.status(h1.task_id).await.unwrap();
        assert_eq!(s1, TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn active_count() {
        let runner = local_runner();
        assert_eq!(runner.active_count().await, 0);

        let task = CloudTask::new("c".into(), "3".into());
        let _h = runner.dispatch(task).await.unwrap();

        // LocalProvider spawns but quickly finishes; check before completion.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let _ = runner.active_count().await; // just assert it doesn't panic
    }
}
