//! Cloud task and status types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// CloudTask
// ---------------------------------------------------------------------------

/// A unit of work dispatched to a cloud backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudTask {
    /// Unique task identifier.
    pub id: Uuid,
    /// The kind of agent task (e.g. `"summarize"`, `"lint"`, `"test"`).
    pub kind: String,
    /// Free-form payload describing what to do.
    pub prompt: String,
    /// Scheduling priority.
    pub priority: TaskPriority,
    /// Optional maximum execution time before the backend kills it.
    pub timeout: Option<std::time::Duration>,
    /// Arbitrary metadata the backend may forward.
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
    /// When the task was created.
    pub created_at: DateTime<Utc>,
}

impl CloudTask {
    /// Create a new task with defaults.
    pub fn new(kind: String, prompt: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            prompt,
            priority: TaskPriority::Normal,
            timeout: None,
            metadata: std::collections::HashMap::new(),
            created_at: Utc::now(),
        }
    }

    /// Builder: set priority.
    pub fn with_priority(mut self, p: TaskPriority) -> Self {
        self.priority = p;
        self
    }

    /// Builder: set timeout.
    pub fn with_timeout(mut self, d: std::time::Duration) -> Self {
        self.timeout = Some(d);
        self
    }

    /// Builder: attach metadata.
    pub fn with_metadata(mut self, key: String, value: serde_json::Value) -> Self {
        self.metadata.insert(key, value);
        self
    }
}

// ---------------------------------------------------------------------------
// TaskPriority
// ---------------------------------------------------------------------------

/// Priority level for cloud task scheduling.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}

// ---------------------------------------------------------------------------
// TaskStatus
// ---------------------------------------------------------------------------

/// Lifecycle status of a dispatched task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// The task has been submitted but not yet picked up.
    Queued,
    /// A worker is actively executing the task.
    Running,
    /// The task finished without errors.
    Completed,
    /// The task finished with an error.
    Failed,
    /// The task was cancelled before completion.
    Cancelled,
    /// The task exceeded its timeout and was killed.
    TimedOut,
}

impl TaskStatus {
    /// Returns `true` if the task is in a terminal state.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        };
        f.write_str(label)
    }
}

// ---------------------------------------------------------------------------
// TaskResult
// ---------------------------------------------------------------------------

/// Final result of a completed (or failed) task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// The task this result belongs to.
    pub task_id: Uuid,
    /// Final status.
    pub status: TaskStatus,
    /// Stdout-like output from the worker (optional).
    pub output: Option<String>,
    /// Error details when the task failed.
    pub error: Option<TaskError>,
    /// Wall-clock execution time reported by the backend.
    pub duration_ms: Option<u64>,
    /// When the result was produced.
    pub finished_at: DateTime<Utc>,
}

impl TaskResult {
    /// Convenience: did the task succeed?
    pub fn is_success(&self) -> bool {
        self.status == TaskStatus::Completed
    }
}

// ---------------------------------------------------------------------------
// TaskError
// ---------------------------------------------------------------------------

/// Structured error produced by a cloud task.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct TaskError {
    /// Short error code (e.g. `"TIMEOUT"`, `"RUNTIME_PANIC"`).
    pub code: String,
    /// Human-readable description.
    pub message: String,
    /// Optional stack trace or diagnostic dump.
    pub stack_trace: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_construction() {
        let task = CloudTask::new("lint".into(), "run clippy".into());
        assert_eq!(task.kind, "lint");
        assert_eq!(task.priority, TaskPriority::Normal);
    }

    #[test]
    fn task_status_terminal() {
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
        assert!(TaskStatus::TimedOut.is_terminal());
        assert!(!TaskStatus::Queued.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
    }

    #[test]
    fn task_status_display() {
        assert_eq!(TaskStatus::Queued.to_string(), "queued");
        assert_eq!(TaskStatus::Running.to_string(), "running");
    }

    #[test]
    fn task_result_success() {
        let result = TaskResult {
            task_id: Uuid::new_v4(),
            status: TaskStatus::Completed,
            output: Some("done".into()),
            error: None,
            duration_ms: Some(123),
            finished_at: Utc::now(),
        };
        assert!(result.is_success());
    }
}
