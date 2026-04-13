use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use derive_setters::Setters;
use ignore::WalkBuilder;
use tokio::task::spawn_blocking;

#[derive(Clone, Debug)]
pub struct File {
    pub path: String,
    pub file_name: Option<String>,
    pub size: u64,
}

impl File {
    pub fn is_dir(&self) -> bool {
        self.path.ends_with('/')
    }
}

#[derive(Debug, Clone, Setters)]
pub struct Walker {
    /// Base directory to start walking from
    cwd: PathBuf,

    /// Maximum depth of directory traversal
    max_depth: usize,

    /// Maximum number of entries per directory
    max_breadth: usize,

    /// Maximum size of individual files to process
    max_file_size: u64,

    /// Maximum number of files to process in total
    max_files: usize,

    /// Maximum total size of all files combined
    max_total_size: u64,

    /// Whether to skip binary files
    skip_binary: bool,
}

const DEFAULT_MAX_FILE_SIZE: u64 = 1024 * 1024; // 1MB
const DEFAULT_MAX_FILES: usize = 100;
const DEFAULT_MAX_TOTAL_SIZE: u64 = 10 * 1024 * 1024; // 10MB
const DEFAULT_MAX_DEPTH: usize = 5;
const DEFAULT_MAX_BREADTH: usize = 10;

impl Walker {
    /// Creates a new Walker instance with all settings set to conservative
    /// values.
    pub fn min_all() -> Self {
        Self {
            cwd: PathBuf::new(),
            max_depth: DEFAULT_MAX_DEPTH,
            max_breadth: DEFAULT_MAX_BREADTH,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            max_files: DEFAULT_MAX_FILES,
            max_total_size: DEFAULT_MAX_TOTAL_SIZE,
            skip_binary: true,
        }
    }

    /// Creates a new Walker instance with all settings set to maximum values.
    /// NOTE: This could produce a large number of files and should be used with
    /// carefully.
    pub fn max_all() -> Self {
        Self {
            cwd: PathBuf::new(),
            max_depth: usize::MAX,
            max_breadth: usize::MAX,
            max_file_size: u64::MAX,
            max_files: usize::MAX,
            max_total_size: u64::MAX,
            skip_binary: false,
        }
    }
}

impl Walker {
    pub async fn get(&self) -> Result<Vec<File>> {
        let walker = self.clone();
        spawn_blocking(move || walker.get_blocking())
            .await
            .context("Failed to spawn blocking task")?
    }

    fn is_likely_binary(path: &std::path::Path) -> bool {
        if let Some(extension) = path.extension() {
            let ext = extension.to_string_lossy().to_lowercase();
            // List of common binary file extensions loaded from file
            let binary_extensions_str = include_str!("binary_extensions.txt");
            let binary_extensions: Vec<&str> = binary_extensions_str
                .lines()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .collect();
            binary_extensions.contains(&ext.as_ref())
        } else {
            false
        }
    }

    /// Blocking function to scan filesystem. Use this when you already have
    /// a runtime or want to avoid spawning a new one.
    pub fn get_blocking(&self) -> Result<Vec<File>> {
        let mut files = Vec::new();
        let mut total_size = 0u64;
        let mut dir_entries: HashMap<String, usize> = HashMap::new();
        let mut file_count = 0;

        // TODO: Convert to async and return a stream
        let walk = WalkBuilder::new(&self.cwd)
            .standard_filters(true) // use standard ignore filters.
            .require_git(false)
            .max_depth(Some(self.max_depth))
            // Skip files that exceed size limit
            .max_filesize(Some(self.max_file_size))
            // TODO: use build_parallel() for better performance
            .build();

        'walk_loop: for entry in walk.flatten() {
            let path = entry.path();

            // Skip symlinks — we only process real files and directories.
            if entry.path_is_symlink() {
                continue;
            }

            // Calculate depth relative to base directory
            let depth = path
                .strip_prefix(&self.cwd)
                .map(|p| p.components().count())
                .unwrap_or(0);

            if depth > self.max_depth {
                continue;
            }

            // Handle breadth limit
            if let Some(parent) = path.parent() {
                let parent_path = parent.to_string_lossy().to_string();
                let entry_count = dir_entries.entry(parent_path).or_insert(0);
                *entry_count += 1;

                if *entry_count > self.max_breadth {
                    continue;
                }
            }

            let is_dir = path.is_dir();

            // Skip binary files if configured
            if self.skip_binary && !is_dir && Self::is_likely_binary(path) {
                continue;
            }

            let metadata = match path.metadata() {
                Ok(meta) => meta,
                Err(_) => continue, // Skip files we can't read metadata for
            };

            let file_size = metadata.len();

            // Check total size limit
            if total_size + file_size > self.max_total_size {
                break 'walk_loop;
            }

            // Check if we've hit the file count limit (only count non-directories)
            if !is_dir {
                file_count += 1;
                if file_count > self.max_files {
                    break 'walk_loop;
                }
            }

            let relative_path = path
                .strip_prefix(&self.cwd)
                .with_context(|| format!("Failed to strip prefix from path: {}", path.display()))?;
            let path_string = relative_path.to_string_lossy().to_string();

            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string());

            // Ensure directory paths end with '/' for is_dir() function
            let path_string = if is_dir {
                format!("{path_string}/")
            } else {
                path_string
            };

            files.push(File { path: path_string, file_name, size: file_size });

            if !is_dir {
                total_size += file_size;
            }
        }

        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self};

    use pretty_assertions::assert_eq;
    use tempfile::{TempDir, tempdir};

    use super::*;

    /// Test Fixtures
    mod fixtures {
        use std::fs::{File, create_dir_all};
        use std::io::Write;

        use super::*;

        pub struct Fixture(TempDir);

        impl Default for Fixture {
            fn default() -> Self {
                let dir = tempdir().expect("Failed to create temp directory");
                Fixture(dir)
            }
        }

        impl Fixture {
            pub fn add_file(&self, name: &str, content: &str) -> Result<()> {
                let file_path = self.0.path().join(name);
                if let Some(parent) = file_path.parent() {
                    create_dir_all(parent)?;
                }
                File::create(file_path.as_path())?.write_all(content.as_bytes())?;
                Ok(())
            }

            pub fn as_path(&self) -> &std::path::Path {
                self.0.path()
            }
        }

        /// Creates a directory with files of specified sizes
        /// Returns a TempDir containing the test files
        pub fn create_sized_files(files: &[(String, u64)]) -> Result<TempDir> {
            let dir = tempdir()?;
            for (name, size) in files {
                let content = vec![b'a'; *size as usize];
                File::create(dir.path().join(name))?.write_all(&content)?;
            }
            Ok(dir)
        }

        /// Creates a directory structure with specified depth and a test file
        /// in each directory Returns a TempDir with nested directories
        /// up to depth
        pub fn create_directory_tree(depth: usize, file_name: &str) -> Result<TempDir> {
            let dir = tempdir()?;
            let mut current = dir.path().to_path_buf();

            for i in 0..depth {
                current = current.join(format!("level{i}"));
                fs::create_dir(&current)?;
                File::create(current.join(file_name))?.write_all(b"test")?;
            }
            Ok(dir)
        }

        /// Creates a directory containing a specified number of files
        /// Returns a tuple of (TempDir, PathBuf) where PathBuf points to the
        /// directory containing files
        pub fn create_file_collection(count: usize, prefix: &str) -> Result<(TempDir, PathBuf)> {
            let dir = tempdir()?;
            let files_dir = dir.path().join("files");
            fs::create_dir(&files_dir)?;

            for i in 0..count {
                File::create(files_dir.join(format!("{prefix}{i}.txt")))?.write_all(b"test")?;
            }
            Ok((dir, files_dir))
        }
    }

    #[tokio::test]
    async fn test_walker_respects_file_size_limit() {
        let fixture = fixtures::create_sized_files(&[
            ("small.txt".into(), 100),
            ("large.txt".into(), DEFAULT_MAX_FILE_SIZE + 100),
        ])
        .unwrap();

        let actual = Walker::min_all()
            .cwd(fixture.path().to_path_buf())
            .get()
            .await
            .unwrap();

        let expected = 1; // Only small.txt should be included
        assert_eq!(
            actual.iter().filter(|f| !f.is_dir()).count(),
            expected,
            "Walker should only include files within size limit"
        );
    }

    #[tokio::test]
    async fn test_walker_filters_binary_files() {
        let fixture =
            fixtures::create_sized_files(&[("text.txt".into(), 10), ("binary.exe".into(), 10)])
                .unwrap();

        let actual = Walker::min_all()
            .cwd(fixture.path().to_path_buf())
            .skip_binary(true)
            .get()
            .await
            .unwrap();

        let expected = vec!["text.txt"];
        let actual_files: Vec<_> = actual
            .iter()
            .filter(|f| !f.is_dir())
            .map(|f| f.path.as_str())
            .collect();

        assert_eq!(
            actual_files, expected,
            "Walker should exclude binary files when skip_binary is true"
        );
    }

    #[tokio::test]
    async fn test_walker_enforces_directory_breadth_limit() {
        let (fixture, _) =
            fixtures::create_file_collection(DEFAULT_MAX_BREADTH + 5, "file").unwrap();

        let actual = Walker::min_all()
            .cwd(fixture.path().to_path_buf())
            .get()
            .await
            .unwrap();

        let expected = DEFAULT_MAX_BREADTH;
        let actual_file_count = actual
            .iter()
            .filter(|f| f.path.starts_with("files/") && !f.is_dir())
            .count();

        assert_eq!(
            actual_file_count, expected,
            "Walker should respect the configured max_breadth limit"
        );
    }

    #[tokio::test]
    async fn test_walker_enforces_directory_depth_limit() {
        let fixture = fixtures::create_directory_tree(DEFAULT_MAX_DEPTH + 3, "test.txt").unwrap();

        let actual = Walker::min_all()
            .cwd(fixture.path().to_path_buf())
            .get()
            .await
            .unwrap();

        let expected = DEFAULT_MAX_DEPTH;
        let actual_max_depth = actual
            .iter()
            .filter(|f| !f.is_dir())
            .map(|f| f.path.split('/').count())
            .max()
            .unwrap();

        assert_eq!(
            actual_max_depth, expected,
            "Walker should respect the configured max_depth limit"
        );
    }

    #[tokio::test]
    async fn test_file_name_and_is_dir() {
        let fixture = fixtures::create_sized_files(&[("test.txt".into(), 100)]).unwrap();

        let actual = Walker::min_all()
            .cwd(fixture.path().to_path_buf())
            .get()
            .await
            .unwrap();

        let file = actual
            .iter()
            .find(|f| !f.is_dir())
            .expect("Should find a file");

        assert_eq!(file.file_name.as_deref(), Some("test.txt"));
        assert!(!file.is_dir());

        let dir = actual
            .iter()
            .find(|f| f.is_dir())
            .expect("Should find a directory");

        assert!(dir.is_dir());
        assert!(dir.path.ends_with('/'));
    }

    #[tokio::test]
    async fn test_walker_respects_ignore_file() {
        let fixture = fixtures::Fixture::default();
        fixture
            .add_file("included/test.rs", "const test: &str = \"include_test\";")
            .unwrap();
        fixture
            .add_file("included/main.rs", "const main: &str = \"include_main\";")
            .unwrap();
        fixture
            .add_file("included/main.log", "included main log content")
            .unwrap();
        fixture
            .add_file("excluded/test.rs", "const test: &str = \"exclude_test\";")
            .unwrap();
        fixture
            .add_file("excluded/main.rs", "const main: &str = \"exclude_main\";")
            .unwrap();
        fixture
            .add_file("excluded/main.log", "excluded main log content")
            .unwrap();
        fixture
            .add_file("base.rs", "const base: &str = \"base\";")
            .unwrap();
        fixture
            .add_file("main.log", "base main log content")
            .unwrap();
        fixture.add_file(".ignore", "excluded/**/*\n*.log").unwrap();

        let actual = Walker::max_all()
            .cwd(fixture.as_path().to_path_buf())
            .get()
            .await
            .unwrap();

        let mut expected = vec!["included/main.rs", "included/test.rs", "base.rs"];
        expected.sort();

        let mut actual_files: Vec<_> = actual
            .iter()
            .filter(|f| !f.is_dir())
            .map(|f| f.path.as_str())
            .collect();
        actual_files.sort();

        assert_eq!(
            actual_files, expected,
            "Walker should exclude files listed in .ignore file"
        );
    }

    #[test]
    fn test_is_likely_binary_detects_binary_files() {
        use std::path::Path;

        // Test known binary extensions
        let binary_files = [
            "program.exe",
            "library.dll",
            "archive.zip",
            "document.pdf",
            "music.mp3",
            "video.mp4",
            "image.bmp",
            "database.sqlite",
            "archive.tar",
            "compressed.gz",
        ];

        for file in &binary_files {
            let path = Path::new(file);
            let actual = Walker::is_likely_binary(path);
            assert!(actual, "File {file} should be detected as binary");
        }
    }

    #[test]
    fn test_is_likely_binary_allows_text_files() {
        use std::path::Path;

        // Test known text extensions
        let text_files = [
            "source.rs",
            "script.js",
            "style.css",
            "markup.html",
            "data.json",
            "config.yaml",
            "readme.md",
            "code.py",
            "program.c",
            "header.h",
        ];

        for file in &text_files {
            let path = Path::new(file);
            let actual = Walker::is_likely_binary(path);
            assert!(!actual, "File {file} should not be detected as binary");
        }
    }

    #[test]
    fn test_is_likely_binary_handles_edge_cases() {
        use std::path::Path;

        // Test files without extensions
        let no_extension_files = ["README", "Makefile", "Dockerfile", "LICENSE"];

        for file in &no_extension_files {
            let path = Path::new(file);
            let actual = Walker::is_likely_binary(path);
            assert!(
                !actual,
                "File without extension {file} should not be detected as binary"
            );
        }

        // Test case sensitivity
        let case_test_files = [
            ("program.EXE", true),
            ("DOCUMENT.PDF", true),
            ("Archive.ZIP", true),
            ("Source.RS", false),
            ("Script.JS", false),
        ];

        for (file, expected) in &case_test_files {
            let path = Path::new(file);
            let actual = Walker::is_likely_binary(path);
            assert_eq!(
                actual, *expected,
                "File {} case sensitivity test failed",
                file
            );
        }
    }

    #[tokio::test]
    async fn test_walker_respects_nested_gitignore() {
        let fixture = fixtures::Fixture::default();

        // Root and nested .gitignore files
        fixture.add_file(".gitignore", "*.log\n").unwrap();
        fixture
            .add_file("frontend/.gitignore", "node_modules/\n")
            .unwrap();

        // Files to exclude
        fixture.add_file("debug.log", "").unwrap();
        fixture
            .add_file("frontend/node_modules/lib/index.js", "")
            .unwrap();

        // Files to include
        fixture.add_file("src/main.rs", "").unwrap();
        fixture.add_file("frontend/src/main.ts", "").unwrap();

        let actual = Walker::max_all()
            .cwd(fixture.as_path().to_path_buf())
            .get()
            .await
            .unwrap();

        let mut actual: Vec<_> = actual
            .iter()
            .filter(|f| !f.is_dir())
            .map(|f| f.path.as_str())
            .collect();
        actual.sort();
        let expected = vec!["frontend/src/main.ts", "src/main.rs"];
        assert_eq!(actual, expected, "should respect nested .gitignore files");
    }

    #[tokio::test]
    async fn test_walker_respects_nested_gitignore_with_git_repo() {
        let fixture = fixtures::Fixture::default();

        // Create a .git directory to simulate a real git repository
        let git_dir = fixture.as_path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        std::fs::write(git_dir.join("config"), "[core]\n").unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        fixture.add_file(".gitignore", "*.log\n").unwrap();
        fixture
            .add_file("frontend/.gitignore", "node_modules/\n")
            .unwrap();

        fixture.add_file("debug.log", "").unwrap();
        fixture
            .add_file("frontend/node_modules/lib/index.js", "")
            .unwrap();
        fixture.add_file("src/main.rs", "").unwrap();
        fixture.add_file("frontend/src/main.ts", "").unwrap();

        let actual = Walker::max_all()
            .cwd(fixture.as_path().to_path_buf())
            .get()
            .await
            .unwrap();

        let mut actual: Vec<_> = actual
            .iter()
            .filter(|f| !f.is_dir())
            .map(|f| f.path.as_str())
            .collect();
        actual.sort();
        let expected = vec!["frontend/src/main.ts", "src/main.rs"];
        assert_eq!(
            actual, expected,
            "should respect nested .gitignore in git repos"
        );
    }

    #[tokio::test]
    async fn test_walker_excludes_symlinks() {
        let fixture = fixtures::Fixture::default();

        // Real file that should appear in results.
        fixture.add_file("real.txt", "content").unwrap();

        // Symlink pointing to the real file — must be excluded.
        let link_path = fixture.as_path().join("link.txt");
        std::os::unix::fs::symlink(fixture.as_path().join("real.txt"), &link_path).unwrap();

        let actual = Walker::max_all()
            .cwd(fixture.as_path().to_path_buf())
            .get()
            .await
            .unwrap();

        let actual_files: Vec<_> = actual
            .iter()
            .filter(|f| !f.is_dir())
            .map(|f| f.path.as_str())
            .collect();

        let expected = vec!["real.txt"];
        assert_eq!(
            actual_files, expected,
            "symlinks should be excluded from walker results"
        );
    }

    #[tokio::test]
    async fn test_walker_excludes_dangling_symlinks() {
        let fixture = fixtures::Fixture::default();

        // Real file that should appear in results.
        fixture.add_file("present.txt", "").unwrap();

        // Dangling symlink — target does not exist.
        let dangling = fixture.as_path().join("dangling.txt");
        std::os::unix::fs::symlink(fixture.as_path().join("ghost.txt"), &dangling).unwrap();

        let actual = Walker::max_all()
            .cwd(fixture.as_path().to_path_buf())
            .get()
            .await
            .unwrap();

        let actual_files: Vec<_> = actual
            .iter()
            .filter(|f| !f.is_dir())
            .map(|f| f.path.as_str())
            .collect();

        let expected = vec!["present.txt"];
        assert_eq!(
            actual_files, expected,
            "dangling symlinks should be excluded from walker results"
        );
    }
}
