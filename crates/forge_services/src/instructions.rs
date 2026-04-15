use std::path::PathBuf;
use std::sync::Arc;

use forge_app::{CommandInfra, CustomInstructionsService, EnvironmentInfra, FileReaderInfra};

/// Discovers and loads `AGENTS.md` instruction files from multiple locations,
/// ordered from broadest to most specific scope:
///
/// 1. **Global** — `~/.forge/AGENTS.md`
/// 2. **Git root to CWD walk** — `AGENTS.md` in every directory from the git
///    repository root down through each intermediate directory to the current
///    working directory (inclusive). This enables layered instructions in
///    monorepos and nested project structures.
/// 3. **Fallback** — When no git root is available (or CWD is outside the git
///    root), only the global path and `<CWD>/AGENTS.md` are checked.
///
/// All discovered files are accumulated (not overridden). Missing files are
/// silently skipped. Results are cached for the lifetime of the service
/// instance.
#[derive(Clone)]
pub struct ForgeCustomInstructionsService<F> {
    infra: Arc<F>,
    cache: tokio::sync::OnceCell<Vec<String>>,
}

impl<F: EnvironmentInfra + FileReaderInfra + CommandInfra> ForgeCustomInstructionsService<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra, cache: Default::default() }
    }

    async fn discover_agents_files(&self) -> Vec<PathBuf> {
        let environment = self.infra.get_environment();
        let global_path = environment.global_agentsmd_path();
        let cwd = environment.cwd.clone();
        let git_root = self.get_git_root().await;

        build_instruction_paths(&global_path, git_root.as_ref(), &cwd)
    }

    async fn get_git_root(&self) -> Option<PathBuf> {
        let output = self
            .infra
            .execute_command(
                "git rev-parse --show-toplevel".to_owned(),
                self.infra.get_environment().cwd,
                true, // silent mode - don't print git output
                None, // no environment variables needed for git command
            )
            .await
            .ok()?;

        if output.success() {
            Some(PathBuf::from(output.stdout.trim()))
        } else {
            None
        }
    }

    async fn init(&self) -> Vec<String> {
        let paths = self.discover_agents_files().await;

        let mut custom_instructions = Vec::new();

        for path in paths {
            if let Ok(content) = self.infra.read_utf8(&path).await {
                custom_instructions.push(content);
            }
        }

        custom_instructions
    }
}

/// Builds the ordered list of `AGENTS.md` paths to check.
///
/// Starts with the global path, then walks every directory from `git_root`
/// down to `cwd` (inclusive). Falls back to global + cwd when no git root is
/// available or when cwd is not under the git root.
fn build_instruction_paths(
    global_path: &PathBuf,
    git_root: Option<&PathBuf>,
    cwd: &PathBuf,
) -> Vec<PathBuf> {
    let mut paths = vec![global_path.clone()];

    if let Some(git_root) = git_root {
        if let Ok(relative) = cwd.strip_prefix(git_root) {
            // Walk from git root through each intermediate directory down to
            // CWD.
            let mut current = git_root.clone();
            push_if_absent(&mut paths, current.join("AGENTS.md"));

            for component in relative.components() {
                current.push(component);
                push_if_absent(&mut paths, current.join("AGENTS.md"));
            }
        } else {
            // CWD is outside git root — fall back to CWD only
            push_if_absent(&mut paths, cwd.join("AGENTS.md"));
        }
    } else {
        // No git root (not in a repo) — fall back to CWD only
        push_if_absent(&mut paths, cwd.join("AGENTS.md"));
    }

    paths
}

/// Pushes `path` into `paths` only if it is not already present.
fn push_if_absent(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

#[async_trait::async_trait]
impl<F: EnvironmentInfra + FileReaderInfra + CommandInfra> CustomInstructionsService
    for ForgeCustomInstructionsService<F>
{
    async fn get_custom_instructions(&self) -> Vec<String> {
        self.cache.get_or_init(|| self.init()).await.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::build_instruction_paths;

    #[test]
    fn test_walk_intermediate_directories() {
        let global = PathBuf::from("/home/user/.forge/AGENTS.md");
        let git_root = PathBuf::from("/repo");
        let cwd = PathBuf::from("/repo/packages/app");

        let actual = build_instruction_paths(&global, Some(&git_root), &cwd);
        let expected = vec![
            PathBuf::from("/home/user/.forge/AGENTS.md"),
            PathBuf::from("/repo/AGENTS.md"),
            PathBuf::from("/repo/packages/AGENTS.md"),
            PathBuf::from("/repo/packages/app/AGENTS.md"),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cwd_equals_git_root() {
        let global = PathBuf::from("/home/user/.forge/AGENTS.md");
        let git_root = PathBuf::from("/repo");
        let cwd = PathBuf::from("/repo");

        let actual = build_instruction_paths(&global, Some(&git_root), &cwd);
        let expected = vec![
            PathBuf::from("/home/user/.forge/AGENTS.md"),
            PathBuf::from("/repo/AGENTS.md"),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_no_git_root() {
        let global = PathBuf::from("/home/user/.forge/AGENTS.md");
        let cwd = PathBuf::from("/some/directory");

        let actual = build_instruction_paths(&global, None, &cwd);
        let expected = vec![
            PathBuf::from("/home/user/.forge/AGENTS.md"),
            PathBuf::from("/some/directory/AGENTS.md"),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cwd_outside_git_root() {
        let global = PathBuf::from("/home/user/.forge/AGENTS.md");
        let git_root = PathBuf::from("/repo");
        let cwd = PathBuf::from("/other/directory");

        let actual = build_instruction_paths(&global, Some(&git_root), &cwd);
        let expected = vec![
            PathBuf::from("/home/user/.forge/AGENTS.md"),
            PathBuf::from("/other/directory/AGENTS.md"),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_deeply_nested_cwd() {
        let global = PathBuf::from("/home/user/.forge/AGENTS.md");
        let git_root = PathBuf::from("/repo");
        let cwd = PathBuf::from("/repo/a/b/c/d/e");

        let actual = build_instruction_paths(&global, Some(&git_root), &cwd);
        let expected = vec![
            PathBuf::from("/home/user/.forge/AGENTS.md"),
            PathBuf::from("/repo/AGENTS.md"),
            PathBuf::from("/repo/a/AGENTS.md"),
            PathBuf::from("/repo/a/b/AGENTS.md"),
            PathBuf::from("/repo/a/b/c/AGENTS.md"),
            PathBuf::from("/repo/a/b/c/d/AGENTS.md"),
            PathBuf::from("/repo/a/b/c/d/e/AGENTS.md"),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_global_path_dedup_when_equals_git_root() {
        // Edge case: global base_path happens to be the git root
        let global = PathBuf::from("/repo/AGENTS.md");
        let git_root = PathBuf::from("/repo");
        let cwd = PathBuf::from("/repo/packages/app");

        let actual = build_instruction_paths(&global, Some(&git_root), &cwd);
        let expected = vec![
            PathBuf::from("/repo/AGENTS.md"),
            PathBuf::from("/repo/packages/AGENTS.md"),
            PathBuf::from("/repo/packages/app/AGENTS.md"),
        ];

        assert_eq!(actual, expected);
    }
}
