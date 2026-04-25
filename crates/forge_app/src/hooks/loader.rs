//! Startup verification and loading logic for external hooks.
//!
//! The `load_and_verify_hooks` function is called once at application startup.
//! It discovers all hook scripts, verifies their integrity against the trust
//! store, and returns only the paths of trusted hooks. The result is cached
//! in memory for the entire session — no further disk I/O occurs during
//! runtime.

use std::path::PathBuf;

use crate::hooks::external::discover_hooks;
use crate::hooks::trust::{HookTrustStatus, TrustStore, relative_hook_path};

/// Discovers hooks for the given event, verifies trust, and returns only
/// the paths of hooks that are safe to execute.
///
/// For each discovered hook:
/// - **Trusted** (hash matches) → included in result
/// - **Untrusted** (unknown script) → skipped with guidance printed
/// - **Tampered** (hash mismatch) → high-danger warning, NOT loaded
/// - **Missing** → skipped
///
/// No interactive prompts — users manage trust via `forge hook trust/delete`.
pub fn load_and_verify_hooks(event_name: &str) -> anyhow::Result<Vec<PathBuf>> {
    let all_hooks = discover_hooks(event_name);
    if all_hooks.is_empty() {
        return Ok(Vec::new());
    }

    let mut trust_store = TrustStore::load()?;
    let mut trusted_hooks = Vec::new();
    let mut count_untrusted = 0usize;
    let mut count_tampered = 0usize;
    let mut store_dirty = false;

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
                count_untrusted += 1;
                tracing::warn!(
                    hook = %relative,
                    "Untrusted hook skipped"
                );
            }
            HookTrustStatus::Tampered { expected, actual } => {
                count_tampered += 1;
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

    // Print a startup summary when there are hooks to report.
    if !trusted_hooks.is_empty() || count_untrusted > 0 || count_tampered > 0 {
        let mut parts = Vec::new();
        if !trusted_hooks.is_empty() {
            parts.push(format!("{} loaded", trusted_hooks.len()));
        }
        if count_untrusted > 0 {
            parts.push(format!("{} untrusted", count_untrusted));
        }
        if count_tampered > 0 {
            parts.push(format!("{} tampered", count_tampered));
        }
        eprintln!("  Hooks: {}", parts.join(", "));

        if count_untrusted > 0 {
            eprintln!(
                "  Use `forge hook trust <path>` to trust, or `forge hook delete <path>` to remove."
            );
        }
    }

    Ok(trusted_hooks)
}
