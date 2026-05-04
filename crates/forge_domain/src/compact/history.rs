//! Compaction history tracking for incremental summarization.
//!
//! Tracks what's already been summarized to avoid redundant information
//! and provide context for future summarization decisions.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Tracks the history of compaction operations to enable incremental
/// summarization and avoid redundant processing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompactionHistory {
    /// Content hashes of past summaries to detect redundancy
    pub summary_hashes: Vec<u64>,

    /// Last seen file versions (path -> hash of content at time of compaction)
    /// Used to skip files that haven't changed since last compaction.
    pub file_versions: HashMap<PathBuf, String>,

    /// Count of successful compactions
    pub compaction_count: usize,

    /// Total tokens reduced across all compactions
    pub total_tokens_reduced: usize,

    /// Total messages reduced across all compactions
    pub total_messages_reduced: usize,
}

impl CompactionHistory {
    /// Creates a new empty compaction history
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a compaction operation
    pub fn record_compaction(
        &mut self,
        summary_hash: u64,
        file_versions: HashMap<PathBuf, String>,
        tokens_reduced: usize,
        messages_reduced: usize,
    ) {
        self.compaction_count += 1;
        self.total_tokens_reduced += tokens_reduced;
        self.total_messages_reduced += messages_reduced;

        // Keep last 10 summary hashes for deduplication
        self.summary_hashes.push(summary_hash);
        if self.summary_hashes.len() > 10 {
            self.summary_hashes.remove(0);
        }

        // Update file versions
        for (path, hash) in file_versions {
            self.file_versions.insert(path, hash);
        }

        // Limit file versions to prevent unbounded growth
        if self.file_versions.len() > 1000 {
            // Remove oldest entries (first 100)
            let keys_to_remove: Vec<_> = self.file_versions.keys().take(100).cloned().collect();
            for key in keys_to_remove {
                self.file_versions.remove(&key);
            }
        }
    }

    /// Checks if a file has changed since the last compaction
    pub fn file_changed_since_last_compaction(&self, path: &PathBuf, current_hash: &str) -> bool {
        self.file_versions
            .get(path)
            .map(|h| h != current_hash)
            .unwrap_or(true) // If not in history, consider it changed
    }

    /// Checks if this summary is redundant with a recent compaction
    pub fn is_summary_redundant(&self, hash: u64) -> bool {
        self.summary_hashes.contains(&hash)
    }

    /// Returns statistics about the compaction history
    pub fn stats(&self) -> CompactionHistoryStats {
        CompactionHistoryStats {
            compaction_count: self.compaction_count,
            total_tokens_reduced: self.total_tokens_reduced,
            total_messages_reduced: self.total_messages_reduced,
            tracked_files: self.file_versions.len(),
        }
    }
}

/// Statistics about compaction history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionHistoryStats {
    /// Number of successful compactions
    pub compaction_count: usize,
    /// Total tokens reduced across all compactions
    pub total_tokens_reduced: usize,
    /// Total messages reduced across all compactions
    pub total_messages_reduced: usize,
    /// Number of files currently tracked
    pub tracked_files: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_compaction() {
        let mut history = CompactionHistory::new();

        let mut file_versions = HashMap::new();
        file_versions.insert(PathBuf::from("src/main.rs"), "abc123".to_string());

        history.record_compaction(12345, file_versions, 5000, 20);

        assert_eq!(history.compaction_count, 1);
        assert_eq!(history.total_tokens_reduced, 5000);
        assert_eq!(history.total_messages_reduced, 20);
        assert!(history.summary_hashes.contains(&12345));
    }

    #[test]
    fn test_file_changed() {
        let mut history = CompactionHistory::new();
        let path = PathBuf::from("src/main.rs");

        // File not in history
        assert!(history.file_changed_since_last_compaction(&path, "abc"));

        // Add to history
        let mut file_versions = HashMap::new();
        file_versions.insert(path.clone(), "abc".to_string());
        history.record_compaction(1, file_versions, 0, 0);

        // Same hash - not changed
        assert!(!history.file_changed_since_last_compaction(&path, "abc"));

        // Different hash - changed
        assert!(history.file_changed_since_last_compaction(&path, "xyz"));
    }

    #[test]
    fn test_summary_redundancy() {
        let mut history = CompactionHistory::new();

        assert!(!history.is_summary_redundant(100));

        history.summary_hashes.push(100);
        assert!(history.is_summary_redundant(100));
        assert!(!history.is_summary_redundant(200));
    }

    #[test]
    fn test_history_bounded_growth() {
        let mut history = CompactionHistory::new();

        // Add 15 summaries (limit is 10)
        for i in 0..15 {
            history.record_compaction(i as u64, HashMap::new(), 0, 0);
        }

        assert_eq!(history.summary_hashes.len(), 10);
        // Should contain hashes 5-14 (oldest removed)
        assert!(history.summary_hashes.contains(&5));
        assert!(!history.summary_hashes.contains(&0));
    }
}
