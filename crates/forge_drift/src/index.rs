use std::collections::{HashMap, HashSet};

use parking_lot::RwLock;
use sha2::{Digest, Sha256};

/// Thread-safe index mapping agent ids → their prompt hashes and word-sets.
pub struct DriftIndex {
    inner: RwLock<HashMap<String, AgentPromptIndex>>,
}

struct AgentPromptIndex {
    prompt_sha: [u8; 32],
    words: HashSet<String>,
}

impl Default for DriftIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl DriftIndex {
    pub fn new() -> Self {
        Self { inner: RwLock::new(HashMap::new()) }
    }

    /// Record a new prompt for the given agent.
    pub fn observe(&self, agent_id: &str, prompt: &str) {
        let sha = Sha256::digest(prompt.as_bytes()).into();
        let words = word_set(prompt);
        let mut w = self.inner.write();
        w.insert(
            agent_id.to_string(),
            AgentPromptIndex { prompt_sha: sha, words },
        );
    }

    /// Compute Jaccard similarity between an incoming prompt and a stored
    /// agent's prompt.
    ///
    /// Returns `None` if the agent does not exist in the index.
    pub fn jaccard(&self, agent_id: &str, prompt: &str) -> Option<f64> {
        let r = self.inner.read();
        let stored = r.get(agent_id)?;
        let incoming = word_set(prompt);
        let intersection = stored.words.intersection(&incoming).count();
        let union = stored.words.union(&incoming).count();
        if union == 0 {
            return Some(1.0);
        }
        Some(intersection as f64 / union as f64)
    }

    /// True if the SHA-256 of `prompt` exactly matches the stored hash for
    /// `agent_id`.
    pub fn is_exact_match(&self, agent_id: &str, prompt: &str) -> bool {
        let r = self.inner.read();
        match r.get(agent_id) {
            Some(stored) => {
                let sha = Sha256::digest(prompt.as_bytes());
                sha.as_slice() == stored.prompt_sha
            }
            None => false,
        }
    }

    /// Remove an agent's record (e.g. on disconnect or deregister).
    pub fn remove(&self, agent_id: &str) {
        self.inner.write().remove(agent_id);
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Tokenize a prompt into lower-cased alphanumeric words.
fn word_set(prompt: &str) -> HashSet<String> {
    prompt
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 3)
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_observe_and_exact_match() {
        let ix = DriftIndex::new();
        ix.observe("alice", "create a user profile endpoint with auth");
        assert!(ix.is_exact_match("alice", "create a user profile endpoint with auth"));
        assert!(!ix.is_exact_match("alice", "create a product listing page"));
    }

    #[test]
    fn test_jaccard_same_prompt() {
        let ix = DriftIndex::new();
        let prompt = "implement database schema for orders table";
        ix.observe("alice", prompt);
        let sim = ix.jaccard("alice", prompt);
        assert!(sim.is_some());
        assert!(
            (sim.unwrap() - 1.0).abs() < 1e-6,
            "expected ~1.0, got {}",
            sim.unwrap()
        );
    }

    #[test]
    fn test_jaccard_no_overlap() {
        let ix = DriftIndex::new();
        ix.observe("alice", "payments processing pipeline stripe integration");
        let sim = ix.jaccard("alice", "getting started with python machine learning");
        assert!(sim.is_some());
        assert!(sim.unwrap() < 0.01, "expected ~0.0, got {}", sim.unwrap());
    }

    #[test]
    fn test_remove() {
        let ix = DriftIndex::new();
        ix.observe("bob", "rust async patterns for network services");
        assert!(ix.is_exact_match("bob", "rust async patterns for network services"));
        ix.remove("bob");
        assert!(!ix.is_exact_match("bob", "rust async patterns for network services"));
    }

    #[test]
    fn test_missing_agent_jaccard() {
        let ix = DriftIndex::new();
        ix.observe("carol", "terraform infrastructure as code");
        assert!(
            ix.jaccard("unknown", "terraform infrastructure as code")
                .is_none()
        );
    }
}
