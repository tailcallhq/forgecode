use std::fmt::{Display, Formatter};
use std::hash::Hasher;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
///
/// Contains details about when the snapshot was created,
/// the original file path, the snapshot location, and file size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unique ID for the file
    pub id: SnapshotId,

    /// Unix timestamp when the snapshot was created
    pub timestamp: Duration,

    /// Original file path that is being processed
    pub path: String,
}

impl Snapshot {
    pub fn create(path: PathBuf) -> anyhow::Result<Self> {
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
        })
    }

    /// Create a hash of a file path for storage
    pub fn path_hash(&self) -> String {
        let mut hasher = fnv_rs::Fnv64::default();
        hasher.write(self.path.as_bytes());
        format!("{:x}", hasher.finish())
    }

    /// Create a snapshot filename from a path and timestamp
    pub fn snapshot_path(&self, cwd: Option<PathBuf>) -> PathBuf {
        // Convert Duration to SystemTime then to a formatted string
        let datetime = UNIX_EPOCH + self.timestamp;
        // Format: YYYY-MM-DD_HH-MM-SS-nnnnnnnnn (including nanoseconds)
        let formatted_time = chrono::DateTime::<chrono::Utc>::from(datetime)
            .format("%Y-%m-%d_%H-%M-%S-%9f")
            .to_string();

        let filename = format!("{formatted_time}.snap");
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
    use super::*;

    #[cfg(unix)]
    #[test]
    fn test_create_with_nonexistent_absolute_path() {
        // Test with a non-existent absolute path
        let nonexistent_path = PathBuf::from("/this/path/does/not/exist/file.txt");
        let snapshot = Snapshot::create(nonexistent_path.clone()).unwrap();

        assert!(!snapshot.id.to_string().is_empty());
        assert!(snapshot.timestamp.as_secs() > 0);
        // Should use the original absolute path since canonicalize fails
        assert_eq!(snapshot.path, nonexistent_path.display().to_string());
    }

    #[test]
    fn test_create_with_nonexistent_relative_path() {
        // Test with a non-existent relative path
        let nonexistent_path = PathBuf::from("nonexistent/file.txt");
        let snapshot = Snapshot::create(nonexistent_path.clone());
        assert!(snapshot.is_err());
    }

    #[cfg(windows)]
    #[test]
    fn test_create_with_nonexistent_absolute_windows_path() {
        // Test with Windows-style absolute path that doesn't exist
        let nonexistent_path = PathBuf::from("C:\\nonexistent\\windows\\path\\file.txt");
        let snapshot = Snapshot::create(nonexistent_path.clone()).unwrap();

        assert!(!snapshot.id.to_string().is_empty());
        assert!(snapshot.timestamp.as_secs() > 0);
        assert_eq!(snapshot.path, nonexistent_path.display().to_string());
    }
}
