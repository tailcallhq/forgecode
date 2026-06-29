/// Configuration for drift detection.
///
/// | Field             | Default  | Effect                                      |
/// |-------------------|----------|---------------------------------------------|
/// | `tier`            | T0       | hash-only                                   |
/// | `threshold`       | 0.80     | similarity above this triggers OverlapAlert |
/// | `approval_mode`   | Alert    | Alert | Auto | Off                          |
/// | `concurrent_limit`| 4        | maximum similar jobs to auto-insert          |
pub use forge_similarity::config::{ApprovalMode, Tier};

#[derive(Debug, Clone)]
pub struct DriftConfig {
    /// Detection tier: T0=hash, T1=hash+word-dist, T2=+embed, T3=+rerank
    pub tier: Tier,
    /// Similarity threshold (0.0–1.0) above which a match is emitted.
    pub threshold: f64,
    /// How the system responds on match.
    pub approval_mode: ApprovalMode,
    /// Maximum number of concurrent auto-inserts when approval_mode = Auto.
    pub concurrent_limit: usize,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            tier: Tier::T0,
            threshold: 0.80,
            approval_mode: ApprovalMode::Alert,
            concurrent_limit: 4,
        }
    }
}
