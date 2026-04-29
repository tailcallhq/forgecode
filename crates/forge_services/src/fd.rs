use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use anyhow::Context;
use async_trait::async_trait;
use forge_app::{CommandInfra, WalkerInfra};
use forge_domain::{IgnorePatternsRepository, WorkspaceId};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use tokio::sync::OnceCell;
use tracing::{info, warn};

use crate::error::Error as ServiceError;
use crate::fd_git::FsGit;
use crate::fd_walker::FdWalker;

pub(crate) static ALLOWED_EXTENSIONS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let extensions_str = include_str!("allowed_extensions.txt");
    extensions_str
        .lines()
        .map(|line| line.trim().to_lowercase())
        .filter(|line| !line.is_empty())
        .collect()
});

/// Returns `true` if `path` carries an extension present in the allowed
/// extensions list.
pub(crate) fn has_allowed_extension(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        ALLOWED_EXTENSIONS.contains(&ext.to_string_lossy().to_lowercase() as &str)
    } else {
        false
    }
}

/// Returns `true` if `path` is a symlink (does not follow the link).
fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Filters relative path strings down to those with an allowed extension,
/// resolves each against `dir_path`, and returns them as absolute `PathBuf`s.
///
/// Symlinks are always excluded regardless of their target or extension, so
/// that the sync pipeline only ever processes real files.
///
/// Returns an error when the filtered list is empty, indicating no indexable
/// source files exist in the workspace.
pub(crate) fn filter_and_resolve(
    dir_path: &Path,
    paths: impl IntoIterator<Item = String>,
) -> anyhow::Result<Vec<PathBuf>> {
    let filtered: Vec<PathBuf> = paths
        .into_iter()
        .map(|p| dir_path.join(&p))
        .filter(|p| !is_symlink(p))
        .filter(|p| has_allowed_extension(p))
        .collect();

    if filtered.is_empty() {
        return Err(ServiceError::NoSourceFilesFound.into());
    }

    Ok(filtered)
}

/// Trait for discovering the list of files in a workspace directory that
/// should be considered for synchronisation.
///
/// Implementations may use different strategies (e.g. `git ls-files` or a
/// plain filesystem walk) to enumerate files. The returned paths are absolute.
#[async_trait]
pub trait FileDiscovery: Send + Sync {
    /// Returns the absolute paths of all files to be indexed under `dir_path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the discovery strategy fails and no files can be
    /// enumerated.
    async fn discover(&self, dir_path: &Path) -> anyhow::Result<Vec<PathBuf>>;
}

/// Discovers workspace files using a `FileDiscovery` implementation and logs
/// progress associated with `workspace_id`.
pub async fn discover_sync_file_paths(
    discovery: &impl FileDiscovery,
    dir_path: &Path,
    workspace_id: &WorkspaceId,
) -> anyhow::Result<Vec<PathBuf>> {
    info!(workspace_id = %workspace_id, "Discovering files for sync");
    let files = discovery.discover(dir_path).await?;
    info!(
        workspace_id = %workspace_id,
        count = files.len(),
        "Files discovered and filtered for sync"
    );
    Ok(files)
}

/// A `FileDiscovery` implementation that routes between `GitFileDiscovery` and
/// `WalkerFileDiscovery`.
///
/// It first attempts git-based discovery. If git is unavailable, returns no
/// files, or fails for any reason it transparently falls back to the filesystem
/// walker so that workspaces without git history are still indexed correctly.
///
/// After the strategy returns, `FdDefault` applies the server's gitignore
/// patterns (fetched on first use via [`IgnorePatternsRepository`] and cached
/// for the process lifetime) to the result. When the server is unreachable or
/// the response cannot be compiled the filter is skipped and a warning is
/// logged, so discovery keeps working offline.
pub struct FdDefault<F> {
    infra: Arc<F>,
    git: FsGit<F>,
    walker: FdWalker<F>,
    matcher: OnceCell<Option<Gitignore>>,
}

impl<F> FdDefault<F> {
    /// Creates a new `FdDefault` using the provided infrastructure for both
    /// the git and walker strategies.
    pub fn new(infra: Arc<F>) -> Self {
        Self {
            git: FsGit::new(infra.clone()),
            walker: FdWalker::new(infra.clone()),
            infra,
            matcher: OnceCell::new(),
        }
    }
}

/// Compiles a [`Gitignore`] from the raw contents of the server's
/// `ignore_patterns.txt` using the same semantics as the server: builder root
/// `/` (non-anchored globs match absolute and relative paths alike), blank /
/// `#`-prefixed lines skipped.
fn build_matcher(contents: &str) -> anyhow::Result<Gitignore> {
    let mut builder = GitignoreBuilder::new("/");
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        builder
            .add_line(None, line)
            .with_context(|| format!("invalid ignore pattern: {line}"))?;
    }
    builder.build().context("failed to build ignore matcher")
}

#[async_trait]
impl<F: CommandInfra + WalkerInfra + IgnorePatternsRepository + 'static> FileDiscovery
    for FdDefault<F>
{
    async fn discover(&self, dir_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
        let files = match self.git.discover(dir_path).await {
            Ok(files) => files,
            Err(err) => {
                warn!(error = ?err, "git-based file discovery failed, falling back to walker");
                self.walker.discover(dir_path).await?
            }
        };

        let Some(matcher) = self
            .matcher
            .get_or_init(|| async {
                match self.infra.list_ignore_patterns().await.and_then(|contents| build_matcher(&contents)) {
                    Ok(gi) => Some(gi),
                    Err(err) => {
                        warn!(error = ?err, "failed to load server ignore patterns; continuing without");
                        None
                    }
                }
            })
            .await
        else {
            return Ok(files);
        };

        Ok(files
            .into_iter()
            .filter(|p| {
                !matcher
                    .matched_path_or_any_parents(p, false)
                    .is_ignore()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;

    use forge_app::{WalkedFile, Walker};
    use forge_domain::CommandOutput;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    /// Test double that answers the three infra traits `FdDefault` depends on.
    ///
    /// * `WalkerInfra::walk` returns `files` verbatim so tests can control the
    ///   post-filter input.
    /// * `CommandInfra::execute_command` always fails, forcing `FdDefault` to
    ///   fall back to the walker path.
    /// * `IgnorePatternsRepository::list_ignore_patterns` returns `patterns`.
    struct MockInfra {
        files: Vec<WalkedFile>,
        patterns: String,
    }

    impl MockInfra {
        fn new(files: Vec<WalkedFile>, patterns: &str) -> Self {
            Self { files, patterns: patterns.to_string() }
        }
    }

    fn walked(path: &str) -> WalkedFile {
        WalkedFile {
            path: path.to_string(),
            file_name: Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string()),
            size: 0,
        }
    }

    #[async_trait]
    impl WalkerInfra for MockInfra {
        async fn walk(&self, _config: Walker) -> anyhow::Result<Vec<WalkedFile>> {
            Ok(self.files.clone())
        }
    }

    #[async_trait]
    impl CommandInfra for MockInfra {
        async fn execute_command(
            &self,
            command: String,
            _working_dir: PathBuf,
            _silent: bool,
            _env_vars: Option<Vec<String>>,
        ) -> anyhow::Result<CommandOutput> {
            Ok(CommandOutput {
                command,
                stdout: String::new(),
                stderr: "not a git repo".to_string(),
                exit_code: Some(128),
            })
        }

        async fn execute_command_raw(
            &self,
            _command: &str,
            _working_dir: PathBuf,
            _env_vars: Option<Vec<String>>,
        ) -> anyhow::Result<std::process::ExitStatus> {
            unreachable!("not used by FdDefault discovery")
        }
    }

    #[async_trait]
    impl IgnorePatternsRepository for MockInfra {
        async fn list_ignore_patterns(&self) -> anyhow::Result<String> {
            Ok(self.patterns.clone())
        }
    }

    #[test]
    fn test_filter_and_resolve_excludes_symlinks() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        // Real file with an allowed extension.
        let real_path = base.join("main.rs");
        File::create(&real_path)
            .unwrap()
            .write_all(b"fn main() {}")
            .unwrap();

        // Symlink pointing to the real file (also carries an allowed extension).
        let link_path = base.join("link.rs");
        std::os::unix::fs::symlink(&real_path, &link_path).unwrap();

        let paths = vec!["main.rs".to_string(), "link.rs".to_string()];
        let actual = filter_and_resolve(base, paths).unwrap();

        let expected = vec![base.join("main.rs")];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_filter_and_resolve_excludes_dangling_symlinks() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        // Real file with an allowed extension (keeps the result non-empty).
        let real_path = base.join("lib.rs");
        File::create(&real_path).unwrap().write_all(b"").unwrap();

        // Dangling symlink — target does not exist.
        let dangling = base.join("missing.rs");
        std::os::unix::fs::symlink(base.join("nonexistent.rs"), &dangling).unwrap();

        let paths = vec!["lib.rs".to_string(), "missing.rs".to_string()];
        let actual = filter_and_resolve(base, paths).unwrap();

        let expected = vec![base.join("lib.rs")];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_filter_and_resolve_excludes_symlinks_to_directories() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        // Real file with an allowed extension.
        let real_path = base.join("src").join("main.rs");
        fs::create_dir_all(real_path.parent().unwrap()).unwrap();
        File::create(&real_path).unwrap().write_all(b"").unwrap();

        // Symlink to a directory — even if it appears as a file path it should
        // be excluded.
        let link_dir = base.join("src_link");
        std::os::unix::fs::symlink(base.join("src"), &link_dir).unwrap();

        let paths = vec!["src/main.rs".to_string(), "src_link".to_string()];
        let actual = filter_and_resolve(base, paths).unwrap();

        // src_link has no allowed extension so it is dropped by the extension
        // filter before symlink detection could be needed, but the real file
        // must always be present.
        let expected = vec![base.join("src/main.rs")];
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_discover_filters_files_matching_server_ignore_patterns() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        // Create every candidate on disk so `is_symlink` returns false and
        // `has_allowed_extension` sees a real extension.
        for rel in [
            "main.rs",
            "lib.rs",
            "node_modules/pkg/index.rs",
            "package-lock.json",
        ] {
            let path = base.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            File::create(&path).unwrap();
        }

        let fixture = FdDefault::new(Arc::new(MockInfra::new(
            vec![
                walked("main.rs"),
                walked("lib.rs"),
                walked("node_modules/pkg/index.rs"),
                walked("package-lock.json"),
            ],
            "node_modules\npackage-lock.json\n",
        )));

        let mut actual = fixture.discover(base).await.unwrap();
        actual.sort();

        let mut expected = vec![base.join("lib.rs"), base.join("main.rs")];
        expected.sort();

        assert_eq!(actual, expected);
    }
}
