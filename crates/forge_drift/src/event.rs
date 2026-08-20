use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Id types
// ---------------------------------------------------------------------------

static NEXT_ALERT: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AlertId(u64);

impl AlertId {
    pub fn next() -> Self {
        Self(NEXT_ALERT.fetch_add(1, Ordering::Relaxed))
    }
}

impl std::fmt::Display for AlertId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for AlertId {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u64>().map(Self)
    }
}

/// Discriminates a tie on similarity by comparing the prompt from both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TieBreakerKey {
    NewerPrompt,
    OlderPrompt,
}

/// Reason a user or the system gave for overriding an overlap alert.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum OverrideReason {
    /// User explicitly dismissed the alert.
    UserDismiss,
    /// User acknowledged (saw-and-proceeded).
    UserAck,
    /// Auto-insert was triggered in auto-approve mode.
    AutoInsert,
    /// Alert is stale (agent already finished).
    Stale,
    /// Override because both agents share the same high-level intent.
    SameIntent,
    /// Override because agents are explicitly coordinating.
    Coordinated,
    /// Override directed by user or external orchestrator.
    UserDirected,
}

// ---------------------------------------------------------------------------
// DriftEvent
// ---------------------------------------------------------------------------

/// An event emitted by the drift detector, observable via
/// `broadcast::Receiver`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum DriftEvent {
    /// Two agent prompts have non-trivial overlap.
    OverlapAlert {
        id: AlertId,
        agent_id: String,
        similarity: f64,
        lane: String,
        prompt_excerpt: String,
    },
    /// An alert was overridden (dismissed, acked, or auto-inserted).
    OverrideApplied { id: AlertId, reason: OverrideReason },
    /// In Auto mode, a system-note prompt was injected on the target agent.
    AutoInsert {
        target_agent: String,
        prompt_excerpt: String,
    },
}

impl DriftEvent {
    pub fn alert_id(&self) -> Option<AlertId> {
        match self {
            DriftEvent::OverlapAlert { id, .. } | DriftEvent::OverrideApplied { id, .. } => {
                Some(*id)
            }
            DriftEvent::AutoInsert { .. } => None,
        }
    }
}
