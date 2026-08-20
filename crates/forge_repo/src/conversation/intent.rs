//! ADR-103: Intent state machine for semantic pruning lifecycle
//!
//! State transitions follow a forward-only DAG (no cycles):
//! pending → extracting → extracted → verified → pruned
//!
//! Key invariants:
//! - A conversation can only transition to 'pruned' if current state is
//!   'verified'
//! - All transitions are recorded in audit trail
//! - No backtracking to earlier states (except manual override with operator
//!   approval)

use std::str::FromStr;

/// Intent state in the conversation extraction lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IntentState {
    /// Conversation waiting for extraction batch run
    Pending,
    /// Currently being processed; locked from other extraction runs
    Extracting,
    /// Extraction + MemoryPort.store() succeeded
    Extracted,
    /// Verification confirmed; intent ready for pruning
    Verified,
    /// Context blob compressed or nulled; conversation marked as cold
    Pruned,
}

impl IntentState {
    /// Return the canonical TEXT value for database storage
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Extracting => "extracting",
            Self::Extracted => "extracted",
            Self::Verified => "verified",
            Self::Pruned => "pruned",
        }
    }

    /// Check if a transition from this state to `next` is allowed
    ///
    /// Enforces the forward-only DAG:
    /// - pending → extracting, extracted (skip extraction if needed), verified
    ///   (manual override)
    /// - extracting → extracted, pending (revert on failure)
    /// - extracted → verified, pending (revert on failure)
    /// - verified → pruned, pending (manual revert only)
    /// - pruned → (no forward transitions; pruned conversations are final)
    pub fn can_transition_to(&self, next: IntentState) -> bool {
        match (self, next) {
            // Pending transitions
            (Self::Pending, Self::Extracting) => true, // Normal: start extraction
            (Self::Pending, Self::Extracted) => true,  // Skip extracting (edge case)
            (Self::Pending, Self::Verified) => true,   // Manual override
            (Self::Pending, Self::Pending) => true,    // Idempotent
            // Extracting transitions
            (Self::Extracting, Self::Extracted) => true, // Extraction succeeded
            (Self::Extracting, Self::Pending) => true,   // Revert on failure
            (Self::Extracting, Self::Extracting) => true, // Idempotent (extend lock)
            // Extracted transitions
            (Self::Extracted, Self::Verified) => true, // Verification succeeded
            (Self::Extracted, Self::Pending) => true,  // Revert on verification failure
            (Self::Extracted, Self::Extracted) => true, // Idempotent
            // Verified transitions
            (Self::Verified, Self::Pruned) => true, // Normal: prune
            (Self::Verified, Self::Pending) => true, // Manual revert (operator approval)
            (Self::Verified, Self::Verified) => true, // Idempotent
            // Pruned transitions (no forward; final state)
            (Self::Pruned, Self::Pruned) => true, // Idempotent
            // All other transitions are forbidden
            _ => false,
        }
    }
}

impl FromStr for IntentState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "extracting" => Ok(Self::Extracting),
            "extracted" => Ok(Self::Extracted),
            "verified" => Ok(Self::Verified),
            "pruned" => Ok(Self::Pruned),
            unknown => Err(anyhow::anyhow!(
                "Unknown intent state: '{}'. Expected: pending, extracting, extracted, verified, or pruned",
                unknown
            )),
        }
    }
}

impl std::fmt::Display for IntentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<IntentState> for String {
    fn from(state: IntentState) -> Self {
        state.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_state_from_str() {
        assert_eq!(
            IntentState::from_str("pending").unwrap(),
            IntentState::Pending
        );
        assert_eq!(
            IntentState::from_str("extracting").unwrap(),
            IntentState::Extracting
        );
        assert_eq!(
            IntentState::from_str("extracted").unwrap(),
            IntentState::Extracted
        );
        assert_eq!(
            IntentState::from_str("verified").unwrap(),
            IntentState::Verified
        );
        assert_eq!(
            IntentState::from_str("pruned").unwrap(),
            IntentState::Pruned
        );
        assert!(IntentState::from_str("invalid").is_err());
    }

    #[test]
    fn test_intent_state_as_str() {
        assert_eq!(IntentState::Pending.as_str(), "pending");
        assert_eq!(IntentState::Extracting.as_str(), "extracting");
        assert_eq!(IntentState::Extracted.as_str(), "extracted");
        assert_eq!(IntentState::Verified.as_str(), "verified");
        assert_eq!(IntentState::Pruned.as_str(), "pruned");
    }

    #[test]
    fn test_intent_state_display() {
        assert_eq!(IntentState::Pending.to_string(), "pending");
        assert_eq!(IntentState::Extracting.to_string(), "extracting");
        assert_eq!(IntentState::Extracted.to_string(), "extracted");
        assert_eq!(IntentState::Verified.to_string(), "verified");
        assert_eq!(IntentState::Pruned.to_string(), "pruned");
    }

    #[test]
    fn test_can_transition_to_valid_transitions() {
        // Pending → Extracting
        assert!(IntentState::Pending.can_transition_to(IntentState::Extracting));
        // Extracting → Extracted
        assert!(IntentState::Extracting.can_transition_to(IntentState::Extracted));
        // Extracted → Verified
        assert!(IntentState::Extracted.can_transition_to(IntentState::Verified));
        // Verified → Pruned
        assert!(IntentState::Verified.can_transition_to(IntentState::Pruned));
    }

    #[test]
    fn test_can_transition_to_idempotent() {
        // All states can transition to themselves
        assert!(IntentState::Pending.can_transition_to(IntentState::Pending));
        assert!(IntentState::Extracting.can_transition_to(IntentState::Extracting));
        assert!(IntentState::Extracted.can_transition_to(IntentState::Extracted));
        assert!(IntentState::Verified.can_transition_to(IntentState::Verified));
        assert!(IntentState::Pruned.can_transition_to(IntentState::Pruned));
    }

    #[test]
    fn test_can_transition_to_reversions() {
        // Extracting → Pending (revert on failure)
        assert!(IntentState::Extracting.can_transition_to(IntentState::Pending));
        // Extracted → Pending (revert on verification failure)
        assert!(IntentState::Extracted.can_transition_to(IntentState::Pending));
        // Verified → Pending (manual revert)
        assert!(IntentState::Verified.can_transition_to(IntentState::Pending));
    }

    #[test]
    fn test_can_transition_to_forward_skip() {
        // Pending → Extracted (skip extracting)
        assert!(IntentState::Pending.can_transition_to(IntentState::Extracted));
        // Pending → Verified (manual override)
        assert!(IntentState::Pending.can_transition_to(IntentState::Verified));
    }

    #[test]
    fn test_can_transition_to_forbidden_transitions() {
        // Pending → Pruned (must go through verified)
        assert!(!IntentState::Pending.can_transition_to(IntentState::Pruned));
        // Extracting → Pruned (must go through verified)
        assert!(!IntentState::Extracting.can_transition_to(IntentState::Pruned));
        // Extracted → Pruned (must go through verified)
        assert!(!IntentState::Extracted.can_transition_to(IntentState::Pruned));
        // Pruned → Extracting (pruned is final)
        assert!(!IntentState::Pruned.can_transition_to(IntentState::Extracting));
        // Pruned → Verified (pruned is final)
        assert!(!IntentState::Pruned.can_transition_to(IntentState::Verified));
    }

    #[test]
    fn test_can_transition_enforces_dag() {
        // Verify the key ADR-103 invariant: pruned is only reachable from verified
        let mut can_reach_pruned = vec![];
        for state in [
            IntentState::Pending,
            IntentState::Extracting,
            IntentState::Extracted,
            IntentState::Verified,
            IntentState::Pruned,
        ] {
            if state.can_transition_to(IntentState::Pruned) {
                can_reach_pruned.push(state);
            }
        }
        assert_eq!(
            can_reach_pruned,
            vec![IntentState::Verified, IntentState::Pruned]
        );
    }
}
