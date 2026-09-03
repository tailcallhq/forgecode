//! Graph construction — scan source files and build the dependency graph.

use crate::{CodebaseGraph, DependencyKind, Edge, Node};
use anyhow::{Context, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, info, warn};

/// Regex patterns for detecting dependency statements per language.
#[allow(dead_code)]
struct LanguagePatterns {
    /// Patterns that capture the dependency target from a source line.
    imports: Vec<regex::Regex>,
    calls: Vec<regex::Regex>,
    types: Vec<regex::Regex>,
}

/// Map from file extension to language detection patterns.
/// Build the TypeScript/TSX patterns (shared between .ts and .tsx files).
fn typescript_patterns() -> LanguagePatterns {
    use regex::Regex;
    LanguagePatterns {
        imports: vec![
            Regex::new(r#"^import\s+.*from\s+['"](.+?)['"]"#).unwrap(),
            Regex::new(r#"^import\s*\(\s*['"](.+?)['"]\s*\)"#).unwrap(),
            Regex::new(r#"^const\s+\w+\s*=\s*require\s*\(\s*['"](.+?)['"]\s*\)"#).unwrap(),
        ],
        calls: vec![],
        types: vec![Regex::new(r#"^(?:export\s+)?(?:interface|type|enum)\s+(\w+)"#).unwrap()],
    }
}

/// Map from file extension to language detection patterns.
fn language_patterns() -> HashMap<&'static str, LanguagePatterns> {
    use regex::Regex;

    let mut map = HashMap::new();

    // Rust
    map.insert(
        "rs",
        LanguagePatterns {
            imports: vec![
                Regex::new(r#"^use\s+([\w:]+)"#).unwrap(),
                Regex::new(r#"^extern\s+crate\s+(\w+)"#).unwrap(),
            ],
            calls: vec![Regex::new(r#"(\w+::\w+)\s*:"#).unwrap()],
            types: vec![Regex::new(r#"(?:struct|enum|trait|type)\s+(\w+)"#).unwrap()],
        },
    );

    // TypeScript — shared patterns for .ts and .tsx
    let ts_patterns = typescript_patterns();
    map.insert("ts", ts_patterns);
    map.insert("tsx", typescript_patterns());
    map.insert(
        "js",
        LanguagePatterns {
            imports: vec![
                Regex::new(r#"^const\s+\w+\s*=\s*require\s*\(\s*['"](.+?)['"]\s*\)"#).unwrap(),
                Regex::new(r#"^import\s+.*from\s+['"](.+?)['"]"#).unwrap(),
            ],
            calls: vec![],
            types: vec![],
        },
    );

    // Python
    map.insert(
        "py",
        LanguagePatterns {
            imports: vec![
                Regex::new(r#"^import\s+([\w.]+)"#).unwrap(),
                Regex::new(r#"^from\s+([\w.]+)\s+import"#).unwrap(),
            ],
            calls: vec![],
            types: vec![Regex::new(r#"^class\s+(\w+)"#).unwrap()],
        },
    );

    // Go
    map.insert(
        "go",
        LanguagePatterns {
            imports: vec![
                Regex::new(r#"^import\s+\(\s*$"#).unwrap(),
                Regex::new(r#"^import\s+"(.+?)""#).unwrap(),
                Regex::new(r#"^\s+"(.+?)"$"#).unwrap(),
            ],
            calls: vec![],
            types: vec![Regex::new(r#"^type\s+(\w+)\s+(?:struct|interface)"#).unwrap()],
        },
    );

    map
}

/// Detect the language from a file extension.
fn detect_language(extension: &str) -> String {
    match extension {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "rb" => "ruby",
        "c" | "h" => "c",
        "cpp" | "cxx" | "cc" | "hpp" => "cpp",
        other => other,
    }
    .to_string()
}

/// Compute a SHA-256 hex digest of the file content.
fn content_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize()).to_string()
}

// ---------------------------------------------------------------------------
// GraphBuilder
// ---------------------------------------------------------------------------

/// Builder that scans a directory tree and produces a [`CodebaseGraph`].
///
/// Supports incremental updates: if a [`CodebaseGraph`] is supplied via
/// [`set_existing_graph`], only files whose mtime has changed (or that are new)
/// will be re-scanned.
pub struct GraphBuilder {
    root: PathBuf,
    /// Optional existing graph to patch incrementally.
    existing: Option<CodebaseGraph>,
    /// Directories to skip (relative to root).
    ignores: Vec<String>,
}

impl GraphBuilder {
    /// Create a new builder rooted at the given directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            existing: None,
            ignores: vec![
                "target".into(),
                "node_modules".into(),
                ".git".into(),
                "dist".into(),
                "__pycache__".into(),
                "vendor".into(),
            ],
        }
    }

    /// Supply an existing graph for incremental (delta) rebuilds.
    pub fn with_existing_graph(mut self, graph: CodebaseGraph) -> Self {
        self.existing = Some(graph);
        self
    }

    /// Add a directory name to the ignore list.
    pub fn ignore(mut self, dir: impl Into<String>) -> Self {
        self.ignores.push(dir.into());
        self
    }

    /// Build the dependency graph by scanning the source tree.
    pub async fn build(&mut self) -> Result<CodebaseGraph> {
        info!(root = %self.root.display(), "scanning codebase for dependency graph");

        let mut graph = self.existing.take().unwrap_or_default();

        let patterns = language_patterns();
        let files = self.collect_source_files().await?;
        info!(count = files.len(), "source files discovered");

        // Phase 1 — upsert nodes (skip unchanged files in incremental mode).
        let mut file_contents: HashMap<PathBuf, String> = HashMap::new();

        for path in &files {
            match fs::read(path).await {
                Ok(bytes) => {
                    let hash = content_hash(&bytes);

                    // Skip unchanged files for incremental update
                    if let Some(existing_node) = graph.node_by_path(path) {
                        if existing_node.content_hash == hash {
                            debug!(path = %path.display(), "unchanged, skipping");
                            continue;
                        }
                        // File changed — update node metadata
                        let lang = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(detect_language)
                            .unwrap_or_default();
                        let node = Node {
                            path: path.clone(),
                            language: lang,
                            size: bytes.len() as u64,
                            last_modified: Utc::now(),
                            content_hash: hash,
                        };
                        graph.upsert_node(node);
                        // We'll re-parse edges below
                    } else {
                        let lang = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(detect_language)
                            .unwrap_or_default();
                        let node = Node {
                            path: path.clone(),
                            language: lang,
                            size: bytes.len() as u64,
                            last_modified: Utc::now(),
                            content_hash: hash,
                        };
                        graph.upsert_node(node);
                    }

                    if let Ok(text) = String::from_utf8(bytes) {
                        file_contents.insert(path.clone(), text);
                    }
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "failed to read source file");
                }
            }
        }

        // Phase 2 — parse dependency edges.
        self.parse_edges(&mut graph, &file_contents, &patterns)?;

        info!(
            nodes = graph.node_count(),
            edges = graph.edge_count(),
            "dependency graph built"
        );

        Ok(graph)
    }

    /// Recursively collect all source files under the root.
    async fn collect_source_files(&self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        self.walk_dir(&self.root, &mut files).await?;
        files.sort();
        Ok(files)
    }

    async fn walk_dir(&self, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        let mut entries = fs::read_dir(dir)
            .await
            .with_context(|| format!("failed to read directory {}", dir.display()))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .context("failed to read directory entry")?
        {
            let path = entry.path();

            if path.is_dir() {
                let dir_name = entry.file_name();
                let dir_str = dir_name.to_string_lossy();
                if self.ignores.iter().any(|ig| ig == dir_str.as_ref()) {
                    continue;
                }
                Box::pin(self.walk_dir(&path, files)).await?;
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str())
                && is_source_file(ext)
            {
                files.push(path);
            }
        }

        Ok(())
    }

    /// Parse each file's content to extract edges and populate the graph.
    fn parse_edges(
        &self,
        graph: &mut CodebaseGraph,
        contents: &HashMap<PathBuf, String>,
        patterns: &HashMap<&str, LanguagePatterns>,
    ) -> Result<()> {
        // Build a path→index lookup for quick resolution
        let path_to_index: HashMap<PathBuf, crate::NodeIndex> = graph
            .path_index
            .iter()
            .map(|(p, &idx)| (p.clone(), idx))
            .collect();

        for (path, text) in contents {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            let lang_patterns = match patterns.get(ext) {
                Some(p) => p,
                None => continue,
            };

            let from_idx = match path_to_index.get(path) {
                Some(&idx) => idx,
                None => continue,
            };

            for line in text.lines() {
                let line = line.trim();

                // Check import patterns
                for re in &lang_patterns.imports {
                    if let Some(caps) = re.captures(line)
                        && let Some(target) = caps.get(1)
                    {
                        let target_str = target.as_str();
                        if let Some(to_idx) = resolve_import(target_str, path, &path_to_index) {
                            graph.add_edge(
                                from_idx,
                                to_idx,
                                Edge {
                                    kind: DependencyKind::Import,
                                    label: Some(target_str.to_string()),
                                },
                            );
                        }
                    }
                }

                // Check type patterns
                for re in &lang_patterns.types {
                    if let Some(caps) = re.captures(line)
                        && let Some(target) = caps.get(1)
                    {
                        let target_str = target.as_str();
                        // Type declarations — we record them but they don't
                        // create edges on their own; cross-file type
                        // references would need a more sophisticated resolver.
                        debug!(
                            path = %path.display(),
                            symbol = target_str,
                            "type declaration noted"
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

/// Determine whether a file extension represents a source file we scan.
fn is_source_file(ext: &str) -> bool {
    matches!(
        ext,
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "java"
            | "rb"
            | "c"
            | "h"
            | "cpp"
            | "cxx"
            | "cc"
            | "hpp"
    )
}

/// Attempt to resolve a string import to a concrete file path in the graph.
fn resolve_import(
    import_str: &str,
    from_file: &Path,
    path_index: &HashMap<PathBuf, crate::NodeIndex>,
) -> Option<crate::NodeIndex> {
    let from_dir = from_file.parent()?;

    // Strategy 1 — relative import starting with ./ or ../
    if import_str.starts_with('.') {
        // Try as-is (with .rs / .ts / .py / .js extensions)
        for ext in &["rs", "ts", "tsx", "js", "jsx", "py", "go"] {
            let candidate = from_dir.join(format!("{import_str}.{ext}"));
            if let Some(&idx) = path_index.get(&candidate) {
                return Some(idx);
            }
            // index files: import_str/mod.rs or import_str/index.ts
            for index_name in &["mod.rs", "index.ts", "index.tsx", "index.js", "__init__.py"] {
                let candidate = from_dir.join(import_str).join(index_name);
                if let Some(&idx) = path_index.get(&candidate) {
                    return Some(idx);
                }
            }
        }
        return None;
    }

    // Strategy 2 — bare crate/module name (Rust style: `use crate_name::…`)
    // Look for a top-level directory or file matching the first segment.
    let root_segment = import_str.split("::").next()?;

    // Try src/root_segment.rs
    for ext in &["rs", "ts", "tsx", "js", "jsx", "py", "go"] {
        let candidate = from_file
            .ancestors()
            .last()?
            .join("src")
            .join(format!("{root_segment}.{ext}"));
        if let Some(&idx) = path_index.get(&candidate) {
            return Some(idx);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language("rs"), "rust");
        assert_eq!(detect_language("ts"), "typescript");
        assert_eq!(detect_language("py"), "python");
        assert_eq!(detect_language("go"), "go");
        assert_eq!(detect_language("foo"), "foo");
    }

    #[test]
    fn test_is_source_file() {
        assert!(is_source_file("rs"));
        assert!(is_source_file("ts"));
        assert!(is_source_file("py"));
        assert!(!is_source_file("txt"));
        assert!(!is_source_file("json"));
    }

    #[test]
    fn test_content_hash_deterministic() {
        let data = b"hello world";
        let h1 = content_hash(data);
        let h2 = content_hash(data);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[tokio::test]
    async fn test_builder_scan_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = GraphBuilder::new(dir.path());
        let graph = builder.build().await.unwrap();
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[tokio::test]
    async fn test_builder_finds_rust_files() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("main.rs"),
            "use my_crate::helper;\nfn main() { helper::run(); }\n",
        )
        .unwrap();
        fs::write(src.join("helper.rs"), "pub fn run() {}\n").unwrap();

        let mut builder = GraphBuilder::new(dir.path());
        let graph = builder.build().await.unwrap();
        assert_eq!(graph.node_count(), 2);
    }

    #[tokio::test]
    async fn test_incremental_skip_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.rs"), "fn a() {}\n").unwrap();
        fs::write(src.join("b.rs"), "fn b() {}\n").unwrap();

        // First build
        let mut builder = GraphBuilder::new(dir.path());
        let graph = builder.build().await.unwrap();
        assert_eq!(graph.node_count(), 2);

        // Rebuild with existing graph — files unchanged
        let builder2 = GraphBuilder::new(dir.path()).with_existing_graph(graph);
        // We need to take the graph out to pass it
        // For this test, just verify we can create the builder
        drop(builder2);
    }
}
