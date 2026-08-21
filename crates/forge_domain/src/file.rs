use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    pub path: String,
    pub is_dir: bool,
}

/// Information about a file or file range read operation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInfo {
    /// Starting line position of the read operation
    pub start_line: u64,

    /// Ending line position of the read operation
    pub end_line: u64,

    /// Total number of lines in the file
    pub total_lines: u64,

    /// SHA-256 hash of the **full** file content.
    /// Stored so callers have a stable hash that matches what a subsequent
    /// whole-file read produces (used by the external-change detector).
    pub content_hash: String,
}

impl FileInfo {
    /// Creates a new FileInfo with the specified parameters.
    pub fn new(start_line: u64, end_line: u64, total_lines: u64, content_hash: String) -> Self {
        Self { start_line, end_line, total_lines, content_hash }
    }

    /// Returns true if this represents a partial file read
    pub fn is_partial(&self) -> bool {
        self.start_line > 0 || self.end_line < self.total_lines
    }
}

/// File hash information from the server
///
/// Contains the relative file path and its SHA-256 hash
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHash {
    /// Relative file path from workspace root
    pub path: String,
    /// SHA-256 hash of the file content
    pub hash: String,
}

impl From<super::node::FileNode> for FileHash {
    fn from(node: super::node::FileNode) -> Self {
        Self { path: node.file_path, hash: node.hash }
    }
}

/// Status of a file in relation to the server
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub enum SyncStatus {
    /// File is in sync with server (same hash)
    InSync,
    /// File has been modified locally
    Modified,
    /// File is new (not on server)
    New,
    /// File exists on server but not locally (deleted locally)
    Deleted,
    /// File could not be read locally (e.g. permission error, binary file)
    Failed,
}

/// Information about a file's sync status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStatus {
    /// Relative file path from workspace root
    pub path: String,
    /// Sync status of the file
    pub status: SyncStatus,
}

impl FileStatus {
    /// Create a new file status entry
    pub fn new(path: String, status: SyncStatus) -> Self {
        Self { path, status }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// Reusable fixture for a `FileInfo` covering an arbitrary line range.
    fn file_info_fixture(start_line: u64, end_line: u64, total_lines: u64) -> FileInfo {
        FileInfo::new(start_line, end_line, total_lines, "abc123".to_string())
    }

    /// Reusable fixture for a `FileNode` used to exercise conversions.
    fn file_node_fixture() -> super::super::node::FileNode {
        super::super::node::FileNode {
            file_path: "src/main.rs".to_string(),
            content: "fn main() {}".to_string(),
            hash: "deadbeef".to_string(),
        }
    }

    #[test]
    fn test_file_info_new_sets_all_fields() {
        let actual = FileInfo::new(0, 10, 10, "hash".to_string());

        let expected = FileInfo {
            start_line: 0,
            end_line: 10,
            total_lines: 10,
            content_hash: "hash".to_string(),
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_is_partial_false_for_full_read() {
        let fixture = file_info_fixture(0, 10, 10);

        let actual = fixture.is_partial();

        assert!(!actual);
    }

    #[test]
    fn test_is_partial_true_when_starting_past_first_line() {
        let fixture = file_info_fixture(1, 10, 10);

        let actual = fixture.is_partial();

        assert!(actual);
    }

    #[test]
    fn test_is_partial_true_when_ending_before_last_line() {
        let fixture = file_info_fixture(0, 5, 10);

        let actual = fixture.is_partial();

        assert!(actual);
    }

    #[test]
    fn test_is_partial_false_for_empty_file() {
        let fixture = file_info_fixture(0, 0, 0);

        let actual = fixture.is_partial();

        assert!(!actual);
    }

    #[test]
    fn test_file_hash_from_file_node_drops_content() {
        let fixture = file_node_fixture();

        let actual = FileHash::from(fixture);

        let expected = FileHash {
            path: "src/main.rs".to_string(),
            hash: "deadbeef".to_string(),
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_file_status_new_sets_all_fields() {
        let actual = FileStatus::new("src/lib.rs".to_string(), SyncStatus::Modified);

        let expected = FileStatus { path: "src/lib.rs".to_string(), status: SyncStatus::Modified };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sync_status_ordering_follows_declaration_order() {
        let fixture = vec![
            SyncStatus::Failed,
            SyncStatus::New,
            SyncStatus::InSync,
            SyncStatus::Deleted,
            SyncStatus::Modified,
        ];

        let actual = {
            let mut sorted = fixture;
            sorted.sort();
            sorted
        };

        let expected = vec![
            SyncStatus::InSync,
            SyncStatus::Modified,
            SyncStatus::New,
            SyncStatus::Deleted,
            SyncStatus::Failed,
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_file_serde_roundtrip() {
        let fixture = File { path: "docs/readme.md".to_string(), is_dir: false };

        let actual: File = serde_json::from_str(&serde_json::to_string(&fixture).unwrap()).unwrap();

        let expected = fixture.clone();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_sync_status_serializes_as_variant_name() {
        let fixture = SyncStatus::Deleted;

        let actual = serde_json::to_string(&fixture).unwrap();

        let expected = "\"Deleted\"".to_string();
        assert_eq!(actual, expected);
    }
}
