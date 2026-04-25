//! Startup verification and loading logic for external hooks.
//!
//! The `load_and_verify_hooks` function is called once at application startup.
//! It discovers all hook scripts, verifies their integrity against the trust
//! store, and returns only the paths of trusted hooks. The result is cached
//! in memory for the entire session — no further disk I/O occurs during
//! runtime.

use std::path::PathBuf;
use std::sync::Arc;

use crate::hooks::external::ExternalHookInterceptor;
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
    let all_hooks = ExternalHookInterceptor::discover_hooks(event_name);
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

                let choice = user_infra
                    .select_one_enum::<TrustPromptChoice>(&format!(
                        "Untrusted hook found: {relative}\nDo you trust this hook?",
                    ))
                    .await?;

                match choice {
                    Some(TrustPromptChoice::Trust) => {
                        trust_store.trust(&relative, hook_path)?;
                        store_dirty = true;
                        trusted_hooks.push(hook_path.clone());
                        tracing::info!(hook = %relative, "Hook trusted by user");
                    }
                    Some(TrustPromptChoice::Delete) => {
                        let _ = std::fs::remove_file(hook_path);
                        tracing::info!(hook = %relative, "Hook deleted by user");
                    }
                    Some(TrustPromptChoice::Ignore) | None => {
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
                    "  Re-trust: forge hooks trust {relative}"
                );
                eprintln!(
                    "  Or delete: forge hooks delete {relative}"
                );
                eprintln!();

                // Remove from trust store so it doesn't keep warning on every
                // startup
                trust_store.untrust(&relative);
                store_dirty = true;
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
}
