//! Startup verification and loading logic for external hooks.
//!
//! The `load_and_verify_hooks` function is called once at application startup.
//! It discovers all hook scripts, verifies their integrity against the trust
//! store, and returns only the paths of trusted hooks. The result is cached
//! in memory for the entire session — no further disk I/O occurs during
//! runtime.

use std::path::PathBuf;
use std::sync::Arc;

use crate::hooks::external::discover_hooks;
use crate::hooks::trust::{HookTrustStatus, TrustStore, relative_hook_path};
use crate::infra::UserInfra;

/// Options presented to the user when an untrusted hook is discovered.
#[derive(
    Debug,
    Clone,
    PartialEq,
    strum_macros::EnumIter,
    strum_macros::EnumString,
    strum_macros::Display,
)]
pub enum TrustPromptChoice {
    Trust,
    Delete,
    Ignore,
}

/// Discovers hooks for the given event, verifies trust, and returns only
/// the paths of hooks that are safe to execute.
///
/// For each discovered hook:
/// - **Trusted** (hash matches) → included in result
/// - **Untrusted** (unknown script) → interactive prompt (Trust/Delete/Ignore)
/// - **Tampered** (hash mismatch) → high-danger warning, NOT loaded, removed
///   from trust store
/// - **Missing** → skipped
///
/// In non-interactive mode (piped stdin / no TTY), untrusted hooks are silently
/// skipped.
pub async fn load_and_verify_hooks<U: UserInfra>(
    event_name: &str,
    user_infra: Arc<U>,
) -> anyhow::Result<Vec<PathBuf>> {
    let all_hooks = discover_hooks(event_name);
    if all_hooks.is_empty() {
        return Ok(Vec::new());
    }

    let mut trust_store = TrustStore::load()?;
    let mut trusted_hooks = Vec::new();
    let mut store_dirty = false;

    let is_tty = is_stdin_tty();

    for hook_path in &all_hooks {
        let Some(relative) = relative_hook_path(hook_path) else {
            tracing::warn!(
                hook = %hook_path.display(),
                "Hook is not under ~/.forge/hooks/, skipping"
            );
            continue;
        };

        let status = trust_store.check(&relative, hook_path);

        match status {
            HookTrustStatus::Trusted => {
                tracing::debug!(hook = %relative, "Hook trusted");
                trusted_hooks.push(hook_path.clone());
            }
            HookTrustStatus::Untrusted => {
                if !is_tty {
                    tracing::warn!(
                        hook = %relative,
                        "Untrusted hook skipped (non-interactive mode)"
                    );
                    continue;
                }

                tracing::warn!(
                    hook = %relative,
                    "Untrusted hook discovered"
                );

                let preview = read_script_preview(hook_path, 5);
                let prompt = format!(
                    "Untrusted hook detected!\n\n  \
                     Event: {event_name}\n  \
                     Script: {relative}\n  \
                     Path: {}\n\
                     {preview}\
                     \nDo you trust this hook?",
                    hook_path.display(),
                );

                let choice = user_infra
                    .select_one_enum::<TrustPromptChoice>(&prompt)
                    .await?;

                match choice {
                    Some(TrustPromptChoice::Trust) => {
                        trust_store.trust(&relative, hook_path)?;
                        trust_store.unignore(&relative);
                        store_dirty = true;
                        trusted_hooks.push(hook_path.clone());
                        tracing::info!(hook = %relative, "Hook trusted by user");
                    }
                    Some(TrustPromptChoice::Delete) => {
                        let _ = std::fs::remove_file(hook_path);
                        trust_store.unignore(&relative);
                        store_dirty = true;
                        tracing::info!(hook = %relative, "Hook deleted by user");
                    }
                    Some(TrustPromptChoice::Ignore) | None => {
                        trust_store.ignore(&relative);
                        store_dirty = true;
                        tracing::info!(hook = %relative, "Hook ignored by user");
                    }
                }
            }
            HookTrustStatus::Tampered { expected, actual } => {
                tracing::error!(
                    hook = %relative,
                    expected = %expected.get(..16).unwrap_or(&expected),
                    actual = %actual.get(..16).unwrap_or(&actual),
                    "DANGER: Hook script has been modified!"
                );
                eprintln!();
                eprintln!("  DANGER: Hook script has been modified!");
                eprintln!("    Event:    {event_name}");
                eprintln!("    Hook:     {relative}");
                eprintln!(
                    "    Expected: {}...",
                    expected.get(..16).unwrap_or(&expected)
                );
                eprintln!(
                    "    Actual:   {}...",
                    actual.get(..16).unwrap_or(&actual)
                );
                eprintln!("  This hook will NOT be loaded.");
                eprintln!(
                    "  Re-trust: forge hook trust {relative}"
                );
                eprintln!(
                    "  Or delete: forge hook delete {relative}"
                );
                eprintln!();

                // Remove from trust store and add to ignored set so it
                // doesn't keep warning on every startup.
                trust_store.untrust(&relative);
                trust_store.ignore(&relative);
                store_dirty = true;
            }
            HookTrustStatus::Ignored => {
                tracing::debug!(hook = %relative, "Hook ignored (previously dismissed)");
            }
            HookTrustStatus::Missing => {
                // File was discovered but disappeared — skip
            }
        }
    }

    if store_dirty {
        trust_store.save()?;
    }

    Ok(trusted_hooks)
}

/// Reads the first `n` lines of a script file for preview display.
/// Returns a formatted string with the preview, or empty string on failure.
fn read_script_preview(path: &std::path::Path, n: usize) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = content.lines().take(n).collect();
    if lines.is_empty() {
        return String::new();
    }
    let preview = lines.join("\n    ");
    format!("  Preview:\n    {preview}\n")
}

/// Checks whether stdin is connected to a TTY (interactive terminal).
fn is_stdin_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_trust_prompt_choice_display() {
        let actual = format!(
            "{}, {}, {}",
            TrustPromptChoice::Trust,
            TrustPromptChoice::Delete,
            TrustPromptChoice::Ignore
        );
        let expected = "Trust, Delete, Ignore";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_is_stdin_tty_returns_bool() {
        // Just verify it doesn't panic
        let _ = is_stdin_tty();
    }

    #[test]
    fn test_read_script_preview_shows_first_lines() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("hook.sh");
        std::fs::write(&path, "#!/bin/bash\necho hello\necho world\nline4\nline5\nline6\n").unwrap();

        let actual = read_script_preview(&path, 5);
        let expected = "  Preview:\n    #!/bin/bash\n    echo hello\n    echo world\n    line4\n    line5\n";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_read_script_preview_empty_file() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("hook.sh");
        std::fs::write(&path, "").unwrap();

        let actual = read_script_preview(&path, 5);
        assert_eq!(actual, "");
    }

    #[test]
    fn test_read_script_preview_missing_file() {
        let actual = read_script_preview(std::path::Path::new("/nonexistent/hook.sh"), 5);
        assert_eq!(actual, "");
    }
}
