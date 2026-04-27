//! Trust store and integrity verification for external hook scripts.
//!
//! Provides SHA-256 hash computation, trust status tracking, and a centralized
//! JSON-based trust store (`~/.forge/hooks/trust.json`). Only hooks whose hash
//! matches the stored value are considered trusted and loaded at startup.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Record for a single trusted hook in the trust store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrustedHook {
    /// SHA-256 hex digest of the hook file contents at the time of trust.
    pub sha256: String,
    /// ISO 8601 timestamp when the hook was trusted.
    pub trusted_at: String,
}

/// Trust status of a hook relative to the trust store.
#[derive(Debug, Clone, PartialEq)]
pub enum HookTrustStatus {
    /// Hash matches the stored value — hook is unmodified.
    Trusted,
    /// No entry in the trust store — hook has never been explicitly trusted.
    Untrusted,
    /// Hash mismatch — hook has been modified since it was trusted.
    Tampered { expected: String, actual: String },
    /// The hook file no longer exists on disk.
    Missing,
}

/// Centralized trust store persisted at `~/.forge/hooks/trust.json`.
///
/// Keys are relative paths from `~/.forge/hooks/` (e.g.
/// `toolcall-start.d/01-my-hook.sh`) to keep the store portable.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrustStore {
    /// Schema version for future migrations.
    pub version: u32,
    /// Map from relative path to trusted hook record.
    pub hooks: BTreeMap<String, TrustedHook>,
}

/// Returns the base directory for all hooks: `~/.forge/hooks/`.
pub fn hooks_base_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".forge").join("hooks"))
}

/// Validates that a given path is within the hooks base directory.
///
/// This function canonicalizes both the input path and the base directory,
/// then checks that the canonicalized path starts with the base directory.
///
/// # Errors
///
/// Returns an error if:
/// - The hooks base directory cannot be determined
/// - The path cannot be canonicalized
/// - The path is outside the hooks base directory (path traversal attack)
pub fn validate_hook_path(path: &Path) -> Result<PathBuf> {
    let base = hooks_base_dir().ok_or_else(|| {
        anyhow::anyhow!("Cannot determine hooks base directory")
    })?;

    // Canonicalize both paths to resolve symlinks and normalize
    let canonical_base = base.canonicalize().with_context(|| {
        format!("Failed to canonicalize base directory: {}", base.display())
    })?;

    let canonical_path = path.canonicalize().with_context(|| {
        format!("Failed to canonicalize path: {}", path.display())
    })?;

    // Check if the canonical path is within the base directory
    if !canonical_path.starts_with(&canonical_base) {
        return Err(anyhow::anyhow!(
            "Path traversal detected: {} is outside hooks directory {}",
            canonical_path.display(),
            canonical_base.display()
        ));
    }

    Ok(canonical_path)
}

/// Variant of [`validate_hook_path`] for delete operations where the file may
/// no longer exist on disk.
///
/// If the file exists, behaves identically to [`validate_hook_path`]
/// (canonicalize + traversal check). If the file is missing, falls back to
/// normalizing the path components without resolving symlinks, then verifies
/// the normalized path is within the hooks base directory.
///
/// This allows users to clean up stale trust-store entries for hooks that have
/// already been removed from disk.
///
/// # Errors
///
/// Returns an error if the hooks base directory cannot be determined or the
/// path escapes the hooks directory.
pub fn validate_hook_path_for_delete(path: &Path) -> Result<PathBuf> {
    let base = hooks_base_dir().ok_or_else(|| {
        anyhow::anyhow!("Cannot determine hooks base directory")
    })?;

    // If the file still exists, use the strict canonicalize-based check.
    if path.exists() {
        return validate_hook_path(path);
    }

    // File is gone — normalize without resolving symlinks.
    // canonicalize on a non-existent path would fail, so we use a
    // best-effort normalization instead.
    let canonical_base = base.canonicalize().with_context(|| {
        format!("Failed to canonicalize base directory: {}", base.display())
    })?;

    // Always join to the non-canonical base so the result is compatible
    // with `relative_hook_path`, which strips the non-canonical base.
    let normalized = base.join(path);

    // Lexically normalize the path to resolve `.` and `..` components
    // without touching the filesystem (canonicalize requires existence).
    let lexically_normalized: PathBuf = normalized.components().fold(
        PathBuf::new(),
        |mut acc, comp| {
            match comp {
                std::path::Component::ParentDir => {
                    acc.pop();
                }
                std::path::Component::CurDir => {}
                c => acc.push(c),
            }
            acc
        },
    );

    // For the traversal check, verify the lexically-normalized path
    // is within canonical_base. Since lexically_normalized was built from
    // base.join(path), it always starts with `base` — strip_prefix cannot
    // fail unless there is a logic bug.
    let relative = lexically_normalized
        .strip_prefix(&base)
        .expect("lexically_normalized was built from base.join(path), so it must start with base");
    let canonical_normalized = canonical_base.join(relative);
    if !canonical_normalized.starts_with(&canonical_base) {
        return Err(anyhow::anyhow!(
            "Path traversal detected: {} is outside hooks directory {}",
            lexically_normalized.display(),
            canonical_base.display()
        ));
    }

    Ok(lexically_normalized)
}

/// Returns the path to the trust store file: `~/.forge/hooks/trust.json`.
pub fn trust_store_path() -> Option<PathBuf> {
    hooks_base_dir().map(|dir| dir.join("trust.json"))
}

/// Discovers all event names by scanning `~/.forge/hooks/` for `*.d`
/// directories.
pub fn discover_events() -> Vec<String> {
    let Some(base) = hooks_base_dir() else {
        return Vec::new();
    };

    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };

    let mut events: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name()?.to_str()?.to_string();
                name.strip_suffix(".d").map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();

    events.sort();
    events
}

/// Computes the SHA-256 hash of a file's contents, returned as lowercase hex.
///
/// # Errors
///
/// Returns an error if the file cannot be read.
pub fn compute_file_hash(path: &Path) -> Result<String> {
    let content = std::fs::read(path)
        .with_context(|| format!("Failed to read hook file: {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let result = hasher.finalize();
    Ok(hex::encode(result))
}

/// Returns the relative path of a hook from `~/.forge/hooks/`, or `None` if
/// the base directory cannot be resolved or the path is not under it.
///
/// Tries the non-canonical base first (the common case). If that fails (e.g.
/// `hook_path` was canonicalized and `HOME` is a symlink), falls back to the
/// canonical base so that symlinked home directories still produce correct
/// relative keys.
pub fn relative_hook_path(hook_path: &Path) -> Option<String> {
    let base = hooks_base_dir()?;

    // Fast path: non-canonical base matches (most common)
    if let Some(relative) = hook_path
        .strip_prefix(&base)
        .ok()
        .and_then(|r| r.to_str().map(|s| s.to_string()))
    {
        return Some(relative);
    }

    // Slow path: hook_path may be canonicalized while base is not (e.g. HOME
    // is a symlink). Try stripping the canonical base instead.
    let canonical_base = base.canonicalize().ok()?;
    hook_path
        .strip_prefix(&canonical_base)
        .ok()
        .and_then(|r| r.to_str().map(|s| s.to_string()))
}

impl TrustStore {
    /// Creates an empty trust store.
    pub fn new() -> Self {
        Self {
            version: 1,
            hooks: BTreeMap::new(),
        }
    }

    /// Loads the trust store from disk. Returns an empty store if the file
    /// does not exist. Returns an error only if the file exists but cannot
    /// be parsed.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but contains malformed JSON.
    pub fn load() -> Result<Self> {
        let Some(path) = trust_store_path() else {
            return Ok(Self::new());
        };

        if !path.exists() {
            return Ok(Self::new());
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read trust store: {}", path.display()))?;

        match serde_json::from_str::<Self>(&content) {
            Ok(store) => Ok(store),
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Malformed trust.json, treating all hooks as untrusted. \
                     Old content can be recovered from the file before the next \
                     trust operation overwrites it."
                );
                Ok(Self::new())
            }
        }
    }

    /// Persists the trust store to disk atomically (write to temp, then
    /// rename).
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file cannot
    /// be written.
    pub fn save(&self) -> Result<()> {
        let Some(path) = trust_store_path() else {
            anyhow::bail!("Cannot determine home directory for trust store");
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }

        let json = serde_json::to_string_pretty(self)
            .context("Failed to serialize trust store")?;

        // Atomic write: write to temp file then rename
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &json)
            .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
        std::fs::rename(&tmp_path, &path)
            .with_context(|| format!("Failed to rename {} to {}", tmp_path.display(), path.display()))?;

        // Set read-only permissions as a speed bump against casual modification.
        // Not a security boundary — an attacker with write access can chmod it —
        // but forces a deliberate action rather than accidental overwrite.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444));
        }

        Ok(())
    }

    /// Checks the trust status of a hook given its relative path and actual
    /// file path.
    pub fn check(&self, relative_path: &str, actual_path: &Path) -> HookTrustStatus {
        if !actual_path.exists() {
            return HookTrustStatus::Missing;
        }

        let Some(trusted) = self.hooks.get(relative_path) else {
            return HookTrustStatus::Untrusted;
        };

        match compute_file_hash(actual_path) {
            Ok(actual_hash) => {
                if actual_hash == trusted.sha256 {
                    HookTrustStatus::Trusted
                } else {
                    HookTrustStatus::Tampered {
                        expected: trusted.sha256.clone(),
                        actual: actual_hash,
                    }
                }
            }
            Err(_) => HookTrustStatus::Missing,
        }
    }

    /// Marks a hook as trusted by computing its current hash and saving the
    /// record.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read.
    pub fn trust(&mut self, relative_path: &str, hook_path: &Path) -> Result<()> {
        let hash = compute_file_hash(hook_path)?;
        let now = chrono::Utc::now().to_rfc3339();
        self.hooks.insert(
            relative_path.to_string(),
            TrustedHook {
                sha256: hash,
                trusted_at: now,
            },
        );
        Ok(())
    }

    /// Removes a hook from the trust store.
    pub fn untrust(&mut self, relative_path: &str) {
        self.hooks.remove(relative_path);
    }

    /// Returns all known hooks with their trust records (if any).
    pub fn list(&self) -> Vec<(String, Option<&TrustedHook>)> {
        self.hooks
            .iter()
            .map(|(k, v)| (k.clone(), Some(v)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn fixture() -> TempDir {
        TempDir::new().unwrap()
    }

    fn write_hook(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_compute_file_hash_deterministic() {
        let temp = fixture();
        let path = write_hook(temp.path(), "test.sh", "#!/bin/bash\necho hello\n");

        let actual = compute_file_hash(&path).unwrap();
        let expected = compute_file_hash(&path).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 64);
        assert!(actual.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_compute_file_hash_changes_on_modification() {
        let temp = fixture();
        let path = write_hook(temp.path(), "test.sh", "original content");

        let hash1 = compute_file_hash(&path).unwrap();
        std::fs::write(&path, "modified content").unwrap();
        let hash2 = compute_file_hash(&path).unwrap();

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_trust_store_check_trusted() {
        let temp = fixture();
        let hook_path = write_hook(temp.path(), "hook.sh", "#!/bin/bash\necho ok\n");
        let relative = "hook.sh".to_string();

        let mut store = TrustStore::new();
        store.trust(&relative, &hook_path).unwrap();

        let actual = store.check(&relative, &hook_path);
        let expected = HookTrustStatus::Trusted;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_trust_store_check_untrusted() {
        let temp = fixture();
        let hook_path = write_hook(temp.path(), "hook.sh", "#!/bin/bash\necho ok\n");

        let store = TrustStore::new();
        let actual = store.check("hook.sh", &hook_path);
        let expected = HookTrustStatus::Untrusted;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_trust_store_check_tampered() {
        let temp = fixture();
        let hook_path = write_hook(temp.path(), "hook.sh", "original content");

        let mut store = TrustStore::new();
        store.trust("hook.sh", &hook_path).unwrap();

        // Tamper with the file
        std::fs::write(&hook_path, "malicious content").unwrap();

        let actual = store.check("hook.sh", &hook_path);
        match actual {
            HookTrustStatus::Tampered { expected, actual } => {
                assert_ne!(expected, actual);
                assert_eq!(expected.len(), 64);
                assert_eq!(actual.len(), 64);
            }
            other => panic!("Expected Tampered, got {:?}", other),
        }
    }

    #[test]
    fn test_trust_store_check_missing() {
        let store = TrustStore::new();
        let missing_path = PathBuf::from("/nonexistent/hook.sh");
        let actual = store.check("hook.sh", &missing_path);
        let expected = HookTrustStatus::Missing;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_trust_store_trust_and_untrust() {
        let temp = fixture();
        let hook_path = write_hook(temp.path(), "hook.sh", "content");
        let relative = "hook.sh".to_string();

        let mut store = TrustStore::new();
        store.trust(&relative, &hook_path).unwrap();
        assert!(store.hooks.contains_key(&relative));

        store.untrust(&relative);
        assert!(!store.hooks.contains_key(&relative));
    }

    #[test]
    fn test_trust_store_save_and_load() {
        let temp = fixture();
        let hook_path = write_hook(temp.path(), "hook.sh", "content");

        // Override trust_store_path by using a temp dir via env manipulation
        // Since trust_store_path() uses dirs::home_dir(), we test load/save
        // by creating a TrustStore, saving, and loading manually
        let mut store = TrustStore::new();
        store.trust("hook.sh", &hook_path).unwrap();

        let json = serde_json::to_string_pretty(&store).unwrap();
        let loaded: TrustStore = serde_json::from_str(&json).unwrap();

        let actual = loaded.hooks.get("hook.sh").unwrap().sha256.clone();
        let expected = compute_file_hash(&hook_path).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_relative_hook_path() {
        let base = hooks_base_dir().unwrap();
        let hook_path = base.join("toolcall-start.d").join("01-hook.sh");
        let actual = relative_hook_path(&hook_path);
        let expected = Some("toolcall-start.d/01-hook.sh".to_string());
        assert_eq!(actual, expected);
    }


}
