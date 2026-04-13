use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use forge_app::{CommandInfra, WalkerInfra};
use forge_domain::WorkspaceId;
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

/// Returns `true` if the file at `path` should be excluded based on its name,
/// regardless of extension. This covers lock files and other generated
/// dependency manifest files that are not useful to index.
fn is_ignored_by_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let name_lower = name.to_lowercase();

    // Lock files: *-lock.json, *.lock, *.lockb, *.lock.json, etc.
    if name_lower.ends_with(".lock")
        || name_lower.ends_with(".lockb")
        || name_lower.ends_with("-lock.json")
        || name_lower.ends_with("-lock.yaml")
        || name_lower.ends_with("-lock.yml")
        || name_lower.ends_with(".lock.json")
        || name_lower.ends_with(".lockfile")
        || name == "Package.resolved"
    {
        return true;
    }

    false
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
        .filter(|p| !is_ignored_by_name(p))
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
pub struct FdDefault<F> {
    git: FsGit<F>,
    walker: FdWalker<F>,
}

impl<F> FdDefault<F> {
    /// Creates a new `RoutingFileDiscovery` using the provided infrastructure
    /// for both the git and walker strategies.
    pub fn new(infra: Arc<F>) -> Self {
        Self { git: FsGit::new(infra.clone()), walker: FdWalker::new(infra) }
    }
}

#[async_trait]
impl<F: CommandInfra + WalkerInfra + 'static> FileDiscovery for FdDefault<F> {
    async fn discover(&self, dir_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
        match self.git.discover(dir_path).await {
            Ok(files) => Ok(files),
            Err(err) => {
                warn!(error = ?err, "git-based file discovery failed, falling back to walker");
                self.walker.discover(dir_path).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;

    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

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
}
