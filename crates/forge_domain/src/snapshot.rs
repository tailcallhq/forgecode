use std::fmt::{Display, Formatter};
use std::hash::Hasher;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::UserInputId;

/// A newtype for snapshot IDs, internally using UUID
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SnapshotId(Uuid);

impl SnapshotId {
    /// Create a new random SnapshotId
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Parse a SnapshotId from a string
    pub fn parse(s: &str) -> Option<Self> {
        Uuid::parse_str(s).ok().map(Self)
    }

    /// Get the underlying UUID
    pub fn uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for SnapshotId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for SnapshotId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for SnapshotId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

/// Represents information about a file snapshot
/// Represents information about a file snapshot
///
/// Contains details about when the snapshot was created,
/// the original file path, and the user input that triggered it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unique ID for the snapshot.
    pub id: SnapshotId,

    /// Unix timestamp when the snapshot was created.
    pub timestamp: Duration,

    /// Original file path that is being processed.
    pub path: String,

    /// The user input that triggered this snapshot, used to group all file
    /// changes from a single prompt together for prompt-level undo.
    pub user_input_id: UserInputId,
}

impl Snapshot {
    /// Creates a snapshot for the provided file path, tagged with the
    /// `UserInputId` of the prompt that triggered the file mutation.
    ///
    /// # Arguments
    /// * `path` - Absolute or canonicalizable file path to snapshot.
    /// * `user_input_id` - ID of the user prompt that caused this mutation.
    ///
    /// # Errors
    /// Returns an error when the path is relative and cannot be canonicalized,
    /// or when the current system time is earlier than the Unix epoch.
    pub fn create(path: PathBuf, user_input_id: UserInputId) -> anyhow::Result<Self> {
        let path = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                if path.is_absolute() {
                    path
                } else {
                    anyhow::bail!(
                        "Path must be absolute. Please provide an absolute path starting with '/' (Unix) or 'C:\\' (Windows)"
                    );
                }
            }
        };
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?;

        Ok(Self {
            id: SnapshotId::new(),
            timestamp,
            path: path.display().to_string(),
            user_input_id,
        })
    }

    /// Creates a stable hash of the snapshot path for storage.
    pub fn path_hash(&self) -> String {
        let mut hasher = fnv_rs::Fnv64::default();
        hasher.write(self.path.as_bytes());
        format!("{:x}", hasher.finish())
    }

    /// Creates the snapshot file path relative to the snapshot root.
    ///
    /// The filename encodes both the timestamp (for chronological ordering
    /// during per-file undo) and the `UserInputId` (for grouping all changes
    /// from one prompt during prompt-level undo).
    ///
    /// Format: `<timestamp>__<user_input_id>.snap`
    ///
    /// # Arguments
    /// * `cwd` - Optional snapshot root directory to prepend to the generated
    ///   relative path.
    pub fn snapshot_path(&self, cwd: Option<PathBuf>) -> PathBuf {
        let datetime = UNIX_EPOCH + self.timestamp;
        // Format: YYYY-MM-DD_HH-MM-SS-nnnnnnnnn (including nanoseconds)
        let formatted_time = chrono::DateTime::<chrono::Utc>::from(datetime)
            .format("%Y-%m-%d_%H-%M-%S-%9f")
            .to_string();

        let filename = format!("{}__{}.snap", formatted_time, self.user_input_id);
        let path = PathBuf::from(self.path_hash()).join(PathBuf::from(filename));
        if let Some(cwd) = cwd {
            cwd.join(path)
        } else {
            path
        }
    }
}
#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_create_with_nonexistent_absolute_path() {
        let fixture = PathBuf::from("/this/path/does/not/exist/file.txt");
        let user_input_id = UserInputId::new();

        let actual = Snapshot::create(fixture.clone(), user_input_id).unwrap();

        assert!(!actual.id.to_string().is_empty());
        assert!(actual.timestamp.as_secs() > 0);
        assert_eq!(actual.path, fixture.display().to_string());
        assert_eq!(actual.user_input_id, user_input_id);
    }

    #[test]
    fn test_create_with_nonexistent_relative_path() {
        let fixture = PathBuf::from("nonexistent/file.txt");

        let actual = Snapshot::create(fixture, UserInputId::new());

        assert!(actual.is_err());
    }

    #[test]
    fn test_snapshot_path_encodes_user_input_id() {
        let fixture = PathBuf::from("/some/absolute/file.txt");
        let user_input_id = UserInputId::new();

        let snapshot = Snapshot::create(fixture, user_input_id).unwrap();
        let path = snapshot.snapshot_path(None);
        let filename = path.file_name().unwrap().to_string_lossy();

        assert!(
            filename.contains(&user_input_id.to_string()),
            "filename '{filename}' should contain the user_input_id '{user_input_id}'"
        );
        assert!(filename.ends_with(".snap"));
    }

    #[cfg(windows)]
    #[test]
    fn test_create_with_nonexistent_absolute_windows_path() {
        let fixture = PathBuf::from("C:\\nonexistent\\windows\\path\\file.txt");
        let user_input_id = UserInputId::new();

        let actual = Snapshot::create(fixture.clone(), user_input_id).unwrap();

        assert!(!actual.id.to_string().is_empty());
        assert!(actual.timestamp.as_secs() > 0);
        assert_eq!(actual.path, fixture.display().to_string());
        assert_eq!(actual.user_input_id, user_input_id);
    }
}
