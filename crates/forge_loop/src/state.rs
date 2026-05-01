//! Loop state persistence

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Loop entry persisted to disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopEntry {
    pub id: String,
    pub conversation_id: String,
    pub prompt: String,
    #[serde(rename = "intervalSeconds")]
    pub interval_seconds: u64,
    pub status: String,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "lastRun")]
    pub last_run: Option<DateTime<Utc>>,
    #[serde(rename = "nextRun")]
    pub next_run: Option<DateTime<Utc>>,
}

/// Monitor condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Condition {
    /// Time-based trigger (e.g., "at 09:00")
    Time { expression: String },
    /// Interval-based trigger (e.g., "every 15m")
    Interval { seconds: u64 },
    /// File change trigger
    FileChange { path: String },
    /// Git event trigger
    GitEvent { event: String },
    /// Composite condition (AND/OR)
    Composite {
        operator: String, // "and" or "or"
        conditions: Vec<Condition>,
    },
}

/// Monitor entry persisted to disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorEntry {
    pub id: String,
    pub conversation_id: String,
    pub condition: Condition,
    pub prompt: String,
    pub status: String,
    pub last_triggered: Option<DateTime<Utc>>,
}

/// Root state file structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopState {
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    pub version: u32,
    pub loops: Vec<LoopEntry>,
    pub monitors: Vec<MonitorEntry>,
}

impl Default for LoopState {
    fn default() -> Self {
        Self {
            schema: Some("forge://loop/v1".to_string()),
            version: 1,
            loops: Vec::new(),
            monitors: Vec::new(),
        }
    }
}

impl LoopState {
    /// Get the state file path
    pub fn state_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".forge")
            .join("loop")
            .join("state.json")
    }

    /// Load state from disk
    pub fn load() -> std::io::Result<Self> {
        let path = Self::state_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        serde_json::from_str(&content).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })
    }

    /// Save state to disk
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_state() {
        let state = LoopState::default();
        assert_eq!(state.version, 1);
        assert!(state.loops.is_empty());
        assert!(state.monitors.is_empty());
    }

    #[test]
    fn test_condition_serialization() {
        let condition = Condition::Time { expression: "at 09:00".to_string() };
        let json = serde_json::to_string(&condition).unwrap();
        assert!(json.contains("time"));
        assert!(json.contains("at 09:00"));
    }

    #[test]
    fn test_loop_entry_serialization() {
        let entry = LoopEntry {
            id: "test-id".to_string(),
            conversation_id: "conv-123".to_string(),
            prompt: "continue".to_string(),
            interval_seconds: 300,
            status: "running".to_string(),
            created_at: Utc::now(),
            last_run: None,
            next_run: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("test-id"));
        assert!(json.contains("300"));
    }
}
