//! Shared types used by both forge_similarity and forge_drift.
//!
//! These live here to break the cyclic dependency:
//!   forge_similarity ← (shared types) → forge_drift
//! forge_drift imports forge_similarity::config::Tier and friends.

/// Tier controls the quality of similarity detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Tier {
    /// Hash-only (8-byte BLAKE3 + word-distance for short strings).
    T0,
    /// Hash + word-distance for all lengths (no embedding model).
    T1,
    /// Hash + local embedding when a backend is configured.
    T2,
    /// Hash + hosted embedding provider + optional re-rank.
    T3,
}

/// User-facing approval mode for drift alerts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum ApprovalMode {
    /// Hash-level alerts emitted; T1/T2 suppressed.
    Off,
    /// All alerts emitted; user must override.
    #[default]
    Alert,
    /// T2+ alerts trigger auto-insert into target session.
    Auto,
}

/// Top-level drift-detection config.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DriftConfig {
    /// Similarity tier.
    pub tier: Tier,
    /// Approval mode.
    pub approval: ApprovalMode,
    /// Path to the DriftIndex SQLite database.
    pub db_path: std::path::PathBuf,
    /// Whether embedding (T2/T3) is enabled at all.
    pub embeddings_enabled: bool,
    /// Whether to fall back to a local ONNX model when hosted is down.
    pub local_embeddings_enabled: bool,
    /// Retention window for old observations (default 30 days).
    pub retention_days: u32,
    /// Cosine-similarity threshold for T2/T3 alert (default 0.85).
    pub alert_threshold: f32,
    /// Cosine-similarity threshold for auto-insert (default 0.92).
    pub auto_insert_threshold: f32,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            tier: Tier::T1,
            approval: ApprovalMode::Alert,
            // Use the OS temp dir so the default is portable (there is no
            // `/tmp` on Windows).
            db_path: std::env::temp_dir().join("forge_drift.sqlite"),
            embeddings_enabled: false,
            local_embeddings_enabled: false,
            retention_days: 30,
            alert_threshold: 0.85,
            auto_insert_threshold: 0.92,
        }
    }
}
