use std::path::PathBuf;

use anyhow::Result;
use derive_setters::Setters;
use ignore::WalkBuilder;

use crate::parser;
use crate::types::FileEntry;
use crate::types::{FileSymbols, RepoMap};

/// Build a repository structure map by walking a directory and parsing
/// recognised source files with tree-sitter.
///
/// The builder emits a [`RepoMap`] containing per-file symbol listings for all
/// supported languages (Rust, TypeScript, JavaScript, Python, Go).
#[derive(Debug, Clone, Setters)]
pub struct RepoMapBuilder {
    /// Root directory to scan.
    cwd: PathBuf,

    /// Maximum directory depth (default: unlimited).
    max_depth: Option<usize>,

    /// Maximum files to parse before stopping (default: 500).
    max_files: usize,

    /// Maximum file size in bytes (default: 512 KiB).
    max_file_size: u64,
}

impl Default for RepoMapBuilder {
    fn default() -> Self {
        Self {
            cwd: PathBuf::new(),
            max_depth: None,
            max_files: 500,
            max_file_size: 512 * 1024,
        }
    }
}

impl RepoMapBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the repo map — walk files, parse supported languages, collect
    /// symbols.
    pub fn build(&self) -> Result<RepoMap> {
        let files = self.walk_files()?;
        let results: Vec<anyhow::Result<FileSymbols>> = parser::parse_files(&files);

        let mut file_symbols: Vec<FileSymbols> = Vec::new();
        let mut total_symbols = 0usize;

        for r in results {
            match r {
                Ok(mut fs) => {
                    // Make the path relative for the output display
                    if let Ok(rel) = std::path::Path::new(&fs.path).strip_prefix(&self.cwd) {
                        fs.path = rel.to_string_lossy().to_string();
                    }
                    total_symbols += fs.symbols.len();
                    file_symbols.push(fs);
                }
                Err(e) => {
                    tracing::debug!("Skipping file during repo-map build: {e:#}");
                }
            }
        }

        // Sort files by path for stable output.
        file_symbols.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(RepoMap {
            root: self.cwd.to_string_lossy().to_string(),
            total_files: file_symbols.len(),
            total_symbols,
            files: file_symbols,
        })
    }

    /// Walk the directory tree and collect source files.
    fn walk_files(&self) -> Result<Vec<FileEntry>> {
        let max_depth = self.max_depth;
        let max_file_size = self.max_file_size;
        let max_files = self.max_files;

        let mut collected: Vec<FileEntry> = Vec::new();

        let walk = WalkBuilder::new(&self.cwd)
            .standard_filters(true)
            .hidden(true)
            .require_git(false)
            .max_depth(max_depth)
            .max_filesize(Some(max_file_size))
            .filter_entry(|entry| entry.file_name() != ".git")
            .build();

        for result in walk {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }

            let path = entry.path();
            let path_str = path.to_string_lossy().to_string();

            // Skip if the file extension isn't something we can parse.
            if parser::detect_language(&path_str).is_none() {
                continue;
            }

            let size = match path.metadata() {
                Ok(m) => m.len(),
                Err(_) => continue,
            };

            collected.push(FileEntry { path: path_str, size });

            if collected.len() >= max_files {
                break;
            }
        }

        Ok(collected)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    fn write_file(dir: &std::path::Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_builder_finds_rust_files() {
        let dir = tempdir().unwrap();

        write_file(dir.path(), "src/lib.rs", "fn hello() {}\nstruct World;\n");
        write_file(dir.path(), "README.md", "# Hello");
        write_file(dir.path(), "data.json", "{}");
        write_file(dir.path(), "src/main.rs", "mod lib;\nfn main() {}\n");

        let map = RepoMapBuilder::new()
            .cwd(dir.path().to_path_buf())
            .max_files(100)
            .build()
            .unwrap();

        assert_eq!(map.total_files, 2, "should find 2 Rust files");
        assert_eq!(map.total_symbols, 3, "should find 3 symbols total");

        let paths: Vec<&str> = map.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"src/lib.rs"));
        assert!(paths.contains(&"src/main.rs"));
    }

    #[test]
    fn test_builder_respects_max_files() {
        let dir = tempdir().unwrap();

        for i in 0..10 {
            write_file(dir.path(), &format!("file_{i}.rs"), "fn f() {}");
        }

        let map = RepoMapBuilder::new()
            .cwd(dir.path().to_path_buf())
            .max_files(3)
            .build()
            .unwrap();

        assert_eq!(map.total_files, 3, "should be capped at 3 files");
    }

    #[test]
    fn test_builder_ignores_unsupported_extensions() {
        let dir = tempdir().unwrap();

        write_file(dir.path(), "code.rs", "fn f() {}");
        write_file(dir.path(), "doc.md", "# title");
        write_file(dir.path(), "binary.bin", "not rs content");

        let map = RepoMapBuilder::new()
            .cwd(dir.path().to_path_buf())
            .max_files(100)
            .build()
            .unwrap();

        assert_eq!(map.total_files, 1, "only .rs file should be parsed");
        assert_eq!(map.files[0].path, "code.rs");
    }

    #[test]
    fn test_builder_produces_stable_output() {
        let dir = tempdir().unwrap();

        write_file(dir.path(), "b.rs", "fn b() {}");
        write_file(dir.path(), "a.rs", "fn a() {}");
        write_file(dir.path(), "c.rs", "fn c() {}");

        let map = RepoMapBuilder::new()
            .cwd(dir.path().to_path_buf())
            .max_files(100)
            .build()
            .unwrap();

        // Files should be sorted alphabetically.
        let paths: Vec<&str> = map.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["a.rs", "b.rs", "c.rs"]);
    }

    #[test]
    fn test_text_output_format() {
        let dir = tempdir().unwrap();
        write_file(dir.path(), "lib.rs", "fn greet() {}\nstruct Point;");

        let map = RepoMapBuilder::new()
            .cwd(dir.path().to_path_buf())
            .max_files(100)
            .build()
            .unwrap();

        let text = map.to_text();
        assert!(text.contains("repo-map:"));
        assert!(text.contains("lib.rs"));
        assert!(text.contains("fn"));
        assert!(text.contains("struct"));
        assert!(text.contains("greet"));
        assert!(text.contains("Point"));
    }

    #[test]
    fn test_builder_empty_directory() {
        let dir = tempdir().unwrap();

        let map = RepoMapBuilder::new()
            .cwd(dir.path().to_path_buf())
            .build()
            .unwrap();

        assert_eq!(map.total_files, 0);
        assert_eq!(map.total_symbols, 0);
        assert!(map.files.is_empty());
    }

    #[test]
    fn test_builder_gitignored_files() {
        let dir = tempdir().unwrap();

        write_file(dir.path(), ".gitignore", "ignored.rs\n");
        write_file(dir.path(), "keep.rs", "fn keep() {}");
        write_file(dir.path(), "ignored.rs", "fn ignored() {}");

        let map = RepoMapBuilder::new()
            .cwd(dir.path().to_path_buf())
            .max_files(100)
            .build()
            .unwrap();

        let paths: Vec<&str> = map.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"keep.rs"), "keep.rs should be included");
        assert!(
            !paths.contains(&"ignored.rs"),
            "ignored.rs should be excluded by .gitignore"
        );
    }
}
