use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::file_operation::FileOperation;
use crate::{Todo, TodoItem, TodoStatus};

#[derive(Debug, Clone, Default, Setters, Serialize, Deserialize)]
#[setters(into, strip_option)]
pub struct Metrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,

    /// Holds the last file operation for each file
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub file_operations: HashMap<String, FileOperation>,

    /// Tracks all files that have been read in this session
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pub files_accessed: HashSet<String>,

    /// Tracks all known todos for the session, including historical completed
    /// todos that were removed from active updates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<Todo>,
}

impl Metrics {
    /// Records a file operation, replacing any previous operation for the same
    /// file. Only Read operations are tracked in files_accessed.
    pub fn insert(mut self, path: String, metrics: FileOperation) -> Self {
        // Only track Read operations in files_accessed
        if metrics.tool == crate::ToolKind::Read {
            self.files_accessed.insert(path.clone());
        }
        self.file_operations.insert(path, metrics);
        self
    }

    /// Gets the session duration if tracking has started
    pub fn duration(&self, now: DateTime<Utc>) -> Option<Duration> {
        self.started_at
            .map(|start| (now - start).to_std().unwrap_or_default())
    }

    /// Returns todos currently in pending or in-progress states.
    pub fn get_active_todos(&self) -> Vec<Todo> {
        self.todos
            .iter()
            .filter(|todo| matches!(todo.status, TodoStatus::Pending | TodoStatus::InProgress))
            .cloned()
            .collect()
    }

    /// Returns all known todos, including historical completed todos.
    pub fn get_todos(&self) -> &[Todo] {
        &self.todos
    }

    /// Applies a list of todo changes using content as the matching key.
    ///
    /// For each incoming item:
    /// - If `status` is `cancelled`: remove the matching item (if found).
    /// - If an item with the same content already exists: update its status.
    /// - Otherwise: add a new item with a server-generated ID.
    ///
    /// Completed items that are not mentioned in the incoming list are
    /// preserved in history. Active items (pending / in_progress) that are
    /// not mentioned remain unchanged.
    ///
    /// Returns the list of currently active (pending / in_progress) todos.
    ///
    /// # Errors
    ///
    /// Returns an error if any todo content is empty or exceeds 1000
    /// characters.
    pub fn apply_todo_changes(&mut self, changes: Vec<TodoItem>) -> anyhow::Result<Vec<Todo>> {
        for item in &changes {
            if item.content.trim().is_empty() {
                anyhow::bail!("Todo content cannot be empty");
            }
            if item.content.len() > 1000 {
                anyhow::bail!("Todo content exceeds maximum length of 1000 characters");
            }
        }

        for item in changes {
            if item.status == TodoStatus::Cancelled {
                // Remove the item by content key
                self.todos.retain(|t| t.content != item.content);
            } else if let Some(existing) = self.todos.iter_mut().find(|t| t.content == item.content)
            {
                // Update in-place
                existing.status = item.status;
            } else {
                // Add new item with server-generated ID
                self.todos.push(Todo {
                    id: Uuid::new_v4().to_string(),
                    content: item.content,
                    status: item.status,
                });
            }
        }

        Ok(self.get_active_todos())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::ToolKind;

    #[test]
    fn test_metrics_new() {
        let actual = Metrics::default();
        assert_eq!(actual.file_operations.len(), 0);
    }

    #[test]
    fn test_metrics_record_file_operation() {
        let fixture = Metrics::default()
            .insert(
                "file1.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(10u64)
                    .lines_removed(5u64)
                    .content_hash(Some("hash1".to_string())),
            )
            .insert(
                "file2.rs".to_string(),
                FileOperation::new(ToolKind::Patch)
                    .lines_added(3u64)
                    .lines_removed(2u64)
                    .content_hash(Some("hash2".to_string())),
            )
            .insert(
                "file1.rs".to_string(),
                FileOperation::new(ToolKind::Patch)
                    .lines_added(5u64)
                    .lines_removed(1u64)
                    .content_hash(Some("hash1_v2".to_string())),
            );

        let actual = fixture;

        // Check file1 has the last operation recorded (second add overwrites the first)
        let file1_metrics = actual.file_operations.get("file1.rs").unwrap();
        assert_eq!(file1_metrics.lines_added, 5);
        assert_eq!(file1_metrics.lines_removed, 1);
        assert_eq!(file1_metrics.content_hash, Some("hash1_v2".to_string()));

        // Check file2 has its operation recorded
        let file2_metrics = actual.file_operations.get("file2.rs").unwrap();
        assert_eq!(file2_metrics.lines_added, 3);
        assert_eq!(file2_metrics.lines_removed, 2);
    }


    #[test]
    fn test_files_accessed_only_tracks_reads() {
        let metrics = Metrics::default()
            .insert("file1.rs".to_string(), FileOperation::new(ToolKind::Read))
            .insert(
                "file2.rs".to_string(),
                FileOperation::new(ToolKind::Write).lines_added(10u64),
            )
            .insert("file3.rs".to_string(), FileOperation::new(ToolKind::Read))
            .insert(
                "file3.rs".to_string(),
                FileOperation::new(ToolKind::Patch).lines_added(5u64),
            );

        // Only Read operations should be in files_accessed
        // file3 was read first, then patched - it stays in files_accessed
        assert_eq!(metrics.files_accessed.len(), 2);
        assert!(metrics.files_accessed.contains("file1.rs"));
        assert!(metrics.files_accessed.contains("file3.rs"));
        assert!(!metrics.files_accessed.contains("file2.rs")); // Write only, not in set

        // file_operations should have the last operation for each file
        assert_eq!(metrics.file_operations.len(), 3);
        assert_eq!(
            metrics.file_operations.get("file1.rs").unwrap().tool,
            ToolKind::Read
        );
        assert_eq!(
            metrics.file_operations.get("file2.rs").unwrap().tool,
            ToolKind::Write
        );
        assert_eq!(
            metrics.file_operations.get("file3.rs").unwrap().tool,
            ToolKind::Patch
        );
    }

    fn todo_item(content: &str, status: TodoStatus) -> TodoItem {
        TodoItem { content: content.to_string(), status }
    }

    #[test]
    fn test_apply_todo_changes_adds_new_items() {
        let mut fixture = Metrics::default();

        let actual = fixture
            .apply_todo_changes(vec![
                todo_item("Task A", TodoStatus::Pending),
                todo_item("Task B", TodoStatus::InProgress),
            ])
            .unwrap();

        let expected = [
            fixture
                .todos
                .iter()
                .find(|t| t.content == "Task A")
                .cloned()
                .unwrap(),
            fixture
                .todos
                .iter()
                .find(|t| t.content == "Task B")
                .cloned()
                .unwrap(),
        ];
        assert_eq!(actual.len(), 2);
        assert_eq!(actual[0].content, expected[0].content);
        assert_eq!(actual[1].content, expected[1].content);
    }

    #[test]
    fn test_apply_todo_changes_updates_by_content_key() {
        let mut fixture = Metrics::default();
        fixture
            .apply_todo_changes(vec![todo_item("Task A", TodoStatus::Pending)])
            .unwrap();

        fixture
            .apply_todo_changes(vec![todo_item("Task A", TodoStatus::Completed)])
            .unwrap();

        let actual = fixture.get_todos().to_vec();
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].content, "Task A");
        assert_eq!(actual[0].status, TodoStatus::Completed);
    }

    #[test]
    fn test_apply_todo_changes_cancelled_removes_item() {
        let mut fixture = Metrics::default();
        fixture
            .apply_todo_changes(vec![
                todo_item("Task A", TodoStatus::Pending),
                todo_item("Task B", TodoStatus::Pending),
            ])
            .unwrap();

        fixture
            .apply_todo_changes(vec![todo_item("Task A", TodoStatus::Cancelled)])
            .unwrap();

        let actual = fixture.get_todos().to_vec();
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].content, "Task B");
    }

    #[test]
    fn test_apply_todo_changes_preserves_untouched_items() {
        let mut fixture = Metrics::default();
        fixture
            .apply_todo_changes(vec![
                todo_item("Task A", TodoStatus::Pending),
                todo_item("Task B", TodoStatus::Pending),
            ])
            .unwrap();

        // Only update Task A; Task B should remain untouched
        fixture
            .apply_todo_changes(vec![todo_item("Task A", TodoStatus::InProgress)])
            .unwrap();

        let todos = fixture.get_todos().to_vec();
        assert_eq!(todos.len(), 2);
        let task_a = todos.iter().find(|t| t.content == "Task A").unwrap();
        let task_b = todos.iter().find(|t| t.content == "Task B").unwrap();
        assert_eq!(task_a.status, TodoStatus::InProgress);
        assert_eq!(task_b.status, TodoStatus::Pending);
    }

    #[test]
    fn test_apply_todo_changes_completed_stays_in_history() {
        let mut fixture = Metrics::default();
        fixture
            .apply_todo_changes(vec![
                todo_item("Task A", TodoStatus::InProgress),
                todo_item("Task B", TodoStatus::Pending),
            ])
            .unwrap();

        // Complete Task A — should remain in todos even if not sent again
        fixture
            .apply_todo_changes(vec![todo_item("Task A", TodoStatus::Completed)])
            .unwrap();

        // Only active todos are returned
        let active = fixture.get_active_todos();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].content, "Task B");

        // But Task A is still in the full list
        let all = fixture.get_todos().to_vec();
        assert_eq!(all.len(), 2);
    }
}
