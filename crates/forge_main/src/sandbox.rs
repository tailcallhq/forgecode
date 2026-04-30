use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use bstr::ByteSlice;
use forge_domain::TitleFormat;

use crate::title_display::TitleDisplayExt;

pub struct Sandbox<'a> {
    dir: &'a str,
}

impl<'a> Sandbox<'a> {
    pub fn new(dir: &'a str) -> Self {
        Self { dir }
    }

    /// Handles worktree creation and returns the path to the worktree directory
    pub fn create(&self) -> Result<PathBuf> {
        let worktree_name = self.dir;
        // First check if we're in a git repository
        let git_check = Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .context("Failed to check if current directory is a git repository")?;

        if !git_check.status.success() {
            bail!(
                "Current directory is not inside a git repository. Worktree creation requires a git repository."
            );
        }

        // Get the git root directory
        let git_root_output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("Failed to get git root directory")?;

        if !git_root_output.status.success() {
            bail!("Failed to determine git repository root");
        }

        let git_root = String::from_utf8(git_root_output.stdout)
            .context("Git root path contains invalid UTF-8")?
            .trim()
            .to_string();

        let git_root_path = PathBuf::from(&git_root);

        // Get the parent directory of the git root
        let parent_dir = git_root_path.parent().context(
            "Git repository is at filesystem root - cannot create worktree in parent directory",
        )?;

        // Create the worktree path in the parent directory
        let worktree_path = parent_dir.join(worktree_name);

        // Check if worktree already exists
        if worktree_path.exists() {
            // Check if it's already a git worktree by checking if it has a .git file
            // (worktree marker)
            let git_file = worktree_path.join(".git");
            if git_file.exists() {
                let worktree_check = Command::new("git")
                    .args(["rev-parse", "--is-inside-work-tree"])
                    .current_dir(&worktree_path)
                    .output()
                    .context("Failed to check if target directory is a git worktree")?;

                if worktree_check.status.success() {
                    println!(
                        "{}",
                        TitleFormat::info("Worktree [Reused]")
                            .sub_title(worktree_path.display().to_string())
                            .display()
                    );
                    return worktree_path
                        .canonicalize()
                        .context("Failed to canonicalize worktree path");
                }
            }

            bail!(
                "Directory '{}' already exists but is not a git worktree. Please remove it or choose a different name.",
                worktree_path.display()
            );
        }

        // Check if branch already exists
        let branch_check = Command::new("git")
            .args([
                "rev-parse",
                "--verify",
                &format!("refs/heads/{worktree_name}"),
            ])
            .current_dir(&git_root_path)
            .output()
            .context("Failed to check if branch exists")?;

        let branch_exists = branch_check.status.success();

        // Create the worktree
        let mut worktree_cmd = Command::new("git");
        worktree_cmd.args(["worktree", "add"]);

        if !branch_exists {
            // Create new branch from current HEAD
            worktree_cmd.args(["-b", worktree_name]);
        }

        worktree_cmd.args([worktree_path.to_str().unwrap()]);

        if branch_exists {
            worktree_cmd.arg(worktree_name);
        }

        let worktree_output = worktree_cmd
            .current_dir(&git_root_path)
            .output()
            .context("Failed to create git worktree")?;

        if !worktree_output.status.success() {
            let stderr = worktree_output.stderr.to_str_lossy();
            bail!("Failed to create git worktree: {stderr}");
        }

        println!(
            "{}",
            TitleFormat::info("Worktree [Created]")
                .sub_title(worktree_path.display().to_string())
                .display()
        );

        // Return the canonicalized path
        worktree_path
            .canonicalize()
            .context("Failed to canonicalize worktree path")
    }
}
