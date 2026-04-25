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

/// Summary of hook verification results, returned by `load_and_verify_hooks`
/// for the caller to display at the appropriate time.
#[derive(Debug, Default)]
pub struct HookSummary {
    /// Number of hooks that passed verification and will be loaded.
    pub loaded: usize,
    /// Number of hooks with no trust record (skipped).
    pub untrusted: usize,
    /// Number of hooks whose hash no longer matches (tampered, skipped).
    pub tampered: Vec<String>,
}

impl std::fmt::Display for HookSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.loaded == 0 && self.untrusted == 0 && self.tampered.is_empty() {
            return Ok(());
        }

        let mut parts = Vec::new();
        if self.loaded > 0 {
            parts.push(format!("{} loaded", self.loaded));
        }
        if self.untrusted > 0 {
            parts.push(format!("{} untrusted", self.untrusted));
        }
        if !self.tampered.is_empty() {
            parts.push(format!("{} tampered", self.tampered.len()));
        }
        writeln!(f, "  Hooks: {}", parts.join(", "))?;

        if self.untrusted > 0 {
            writeln!(
                f,
                "  Use `forge hook trust <path>` to trust, or `forge hook delete <path>` to remove."
            )?;
        }

        for msg in &self.tampered {
            write!(f, "{msg}")?;
        }

        Ok(())
    }
}

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
pub fn load_and_verify_hooks(event_name: &str) -> anyhow::Result<(Vec<PathBuf>, HookSummary)> {
    let all_hooks = discover_hooks(event_name);
    if all_hooks.is_empty() {
        return Ok((Vec::new(), HookSummary::default()));
    }

    let mut trust_store = TrustStore::load()?;
    let mut trusted_hooks = Vec::new();
    let mut count_untrusted = 0usize;
    let mut tampered_messages = Vec::new();
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
                let mut msg = String::new();
                msg.push_str("\n");
                msg.push_str("  DANGER: Hook script has been modified!\n");
                msg.push_str(&format!("    Event:    {event_name}\n"));
                msg.push_str(&format!("    Hook:     {relative}\n"));
                msg.push_str(&format!(
                    "    Expected: {}...\n",
                    expected.get(..16).unwrap_or(&expected)
                ));
                msg.push_str(&format!(
                    "    Actual:   {}...\n",
                    actual.get(..16).unwrap_or(&actual)
                ));
                msg.push_str("  This hook will NOT be loaded.\n");
                msg.push_str(&format!("  Re-trust: forge hook trust {relative}\n"));
                msg.push_str(&format!("  Or delete: forge hook delete {relative}\n"));
                msg.push_str("\n");

                tracing::error!(
                    hook = %relative,
                    expected = %expected.get(..16).unwrap_or(&expected),
                    actual = %actual.get(..16).unwrap_or(&actual),
                    "DANGER: Hook script has been modified!"
                );

                tampered_messages.push(msg);

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

    let summary = HookSummary {
        loaded: trusted_hooks.len(),
        untrusted: count_untrusted,
        tampered: tampered_messages,
    };

    Ok((trusted_hooks, summary))
}
