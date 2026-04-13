use std::path::{Path, PathBuf};

use anyhow::Result;
use forge_app::DirectoryReaderInfra;
use forge_fs::ForgeFS;
use forge_walker::Walker;
use futures::StreamExt;
use glob::Pattern;

/// Service for reading multiple files from a directory asynchronously
pub struct ForgeDirectoryReaderService {
    parallel_file_reads: usize,
}

impl ForgeDirectoryReaderService {
    /// Creates a new service with the given concurrency cap for parallel reads.
    ///
    /// # Arguments
    ///
    /// * `parallel_file_reads` - Maximum number of files to read concurrently
    pub fn new(parallel_file_reads: usize) -> Self {
        Self { parallel_file_reads }
    }

    /// Lists all entries in a directory without reading file contents
    /// Returns a vector of tuples containing (entry_path, is_directory)
    /// Much more efficient than read_directory_files for directory listings
    /// Respects .gitignore, .ignore, and other standard ignore files
    async fn list_directory_entries(&self, directory: &Path) -> Result<Vec<(PathBuf, bool)>> {
        // Check if directory exists
        if !ForgeFS::exists(directory) || ForgeFS::is_file(directory) {
            return Ok(vec![]);
        }

        // Use Walker to get entries with gitignore filtering
        let files = Walker::max_all()
            .cwd(directory.to_path_buf())
            .max_depth(1)
            .skip_binary(true)
            .hidden(true)
            .get()
            .await?;

        let mut entries = Vec::new();

        for file in files {
            // Skip root directory entry
            if file.path == "/" {
                continue;
            }

            let file_path = PathBuf::from(file.path);
            let absolute_path = if file_path.is_relative() {
                directory.join(file_path)
            } else {
                file_path
            };
            let is_dir = absolute_path.is_dir();
            entries.push((absolute_path, is_dir));
        }

        Ok(entries)
    }

    /// Reads all files in a directory that match the given filter pattern
    /// Returns a vector of tuples containing (file_path, file_content)
    /// Files are read asynchronously/in parallel for better performance
    async fn read_directory_files(
        &self,
        directory: &Path,
        pattern: Option<&str>,
    ) -> Result<Vec<(PathBuf, String)>> {
        // Check if directory exists
        if !ForgeFS::exists(directory) || ForgeFS::is_file(directory) {
            return Ok(vec![]);
        }

        // Build glob pattern if filter is provided
        let glob_pattern = if let Some(pattern) = pattern {
            Some(Pattern::new(pattern)?)
        } else {
            None
        };

        // Read directory entries
        let mut dir = ForgeFS::read_dir(directory).await?;
        let mut file_paths = Vec::new();

        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();

            // Only process files (not directories)
            if ForgeFS::is_file(&path) {
                // Apply filter if provided
                if let Some(ref pattern) = glob_pattern {
                    if let Some(file_name) = path.file_name().and_then(|n| n.to_str())
                        && pattern.matches(file_name)
                    {
                        file_paths.push(path);
                    }
                } else {
                    file_paths.push(path);
                }
            }
        }

        // Read files in parallel with a concurrency cap to avoid EMFILE errors
        let files = futures::stream::iter(file_paths)
            .map(|path| async move {
                let path_clone = path.clone();
                match ForgeFS::read_to_string(&path).await {
                    Ok(content) => Some((path_clone, content)),
                    Err(_) => None, // Skip files that can't be read
                }
            })
            .buffer_unordered(self.parallel_file_reads)
            .filter_map(std::future::ready)
            .collect::<Vec<_>>()
            .await;

        Ok(files)
    }
}

#[async_trait::async_trait]
impl DirectoryReaderInfra for ForgeDirectoryReaderService {
    async fn list_directory_entries(&self, directory: &Path) -> Result<Vec<(PathBuf, bool)>> {
        self.list_directory_entries(directory).await
    }

    async fn read_directory_files(
        &self,
        directory: &Path,
        pattern: Option<&str>,
    ) -> Result<Vec<(PathBuf, String)>> {
        self.read_directory_files(directory, pattern).await
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    fn write_file(path: &Path, content: &str) {
        fs::write(path, content).unwrap();
    }

    #[tokio::test]
    async fn test_read_directory_files_with_filter() {
        let fixture = tempdir().unwrap();
        write_file(&fixture.path().join("test.md"), "# Markdown content");
        write_file(&fixture.path().join("test.txt"), "Text content");
        write_file(&fixture.path().join("test.rs"), "fn main() {}");

        let actual = ForgeDirectoryReaderService::new(64)
            .read_directory_files(fixture.path(), Some("*.md"))
            .await
            .unwrap();

        let expected = vec![(
            fixture.path().join("test.md"),
            "# Markdown content".to_string(),
        )];
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_read_directory_files_without_filter() {
        let fixture = tempdir().unwrap();
        write_file(&fixture.path().join("file1.txt"), "Content 1");
        write_file(&fixture.path().join("file2.md"), "Content 2");

        let mut actual = ForgeDirectoryReaderService::new(64)
            .read_directory_files(fixture.path(), None)
            .await
            .unwrap();
        actual.sort_by(|(a, _), (b, _)| a.file_name().cmp(&b.file_name()));

        let expected = vec![
            (fixture.path().join("file1.txt"), "Content 1".to_string()),
            (fixture.path().join("file2.md"), "Content 2".to_string()),
        ];
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_read_directory_files_nonexistent_directory() {
        let actual = ForgeDirectoryReaderService::new(64)
            .read_directory_files(Path::new("/nonexistent"), None)
            .await
            .unwrap();

        let expected: Vec<(PathBuf, String)> = vec![];
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_read_directory_files_ignores_subdirectories() {
        let fixture = tempdir().unwrap();
        write_file(&fixture.path().join("test.txt"), "File content");

        let subdir = fixture.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        write_file(&subdir.join("subfile.txt"), "Sub content");

        let actual = ForgeDirectoryReaderService::new(64)
            .read_directory_files(fixture.path(), None)
            .await
            .unwrap();

        let expected = vec![(fixture.path().join("test.txt"), "File content".to_string())];
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_list_directory_entries() {
        let fixture = tempdir().unwrap();
        write_file(&fixture.path().join("file1.txt"), "Content 1");
        write_file(&fixture.path().join("file2.md"), "Content 2");

        let subdir = fixture.path().join("subdir");
        fs::create_dir(&subdir).unwrap();

        let mut actual = ForgeDirectoryReaderService::new(64)
            .list_directory_entries(fixture.path())
            .await
            .unwrap();
        actual.sort_by(|(a, _), (b, _)| a.file_name().cmp(&b.file_name()));

        let expected = vec![
            (fixture.path().join("file1.txt"), false),
            (fixture.path().join("file2.md"), false),
            (fixture.path().join("subdir"), true),
        ];
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_list_directory_entries_nonexistent() {
        let actual = ForgeDirectoryReaderService::new(64)
            .list_directory_entries(Path::new("/nonexistent"))
            .await
            .unwrap();

        let expected: Vec<(PathBuf, bool)> = vec![];
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_list_directory_entries_respects_gitignore() {
        let fixture = tempdir().unwrap();
        write_file(&fixture.path().join("file.txt"), "Content");
        write_file(&fixture.path().join("ignored.log"), "Log");

        let subdir = fixture.path().join("node_modules");
        fs::create_dir(&subdir).unwrap();

        let git_dir = fixture.path().join(".git");
        fs::create_dir(&git_dir).unwrap();

        write_file(&fixture.path().join(".gitignore"), "*.log\nnode_modules/\n");

        let actual = ForgeDirectoryReaderService::new(64)
            .list_directory_entries(fixture.path())
            .await
            .unwrap();

        let expected = vec![(fixture.path().join("file.txt"), false)];
        assert_eq!(actual, expected);
    }
}
