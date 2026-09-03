use std::collections::HashSet;
use sha2::{Digest, Sha256};

use crate::config::Tier;
use crate::{SimilarityError, SimilarityProvider};

// ---------------------------------------------------------------------------
// Hashing helpers
// ---------------------------------------------------------------------------

/// Compute the SHA-256 digest of `s` as a 32-byte array.
fn sha256_hash(s: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hasher.finalize().into()
}

// ---------------------------------------------------------------------------
// T0 — exact hash match
// ---------------------------------------------------------------------------

/// Returns 1.0 if the SHA-256 hashes of `a` and `b` match, 0.0 otherwise.
fn exact_hash_match(a: &str, b: &str) -> f64 {
    f64::from(sha256_hash(a) == sha256_hash(b))
}

// ---------------------------------------------------------------------------
// T1 — Jaccard word-overlap
// ---------------------------------------------------------------------------

/// Returns `|A ∩ B| / |A ∪ B|` where A and B are whitespace-split word sets.
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}

// ---------------------------------------------------------------------------
// Public dispatcher
// ---------------------------------------------------------------------------

/// Compare two strings using the similarity strategy for `tier`.
///
/// | Tier | Strategy                          | Range          |
/// |------|-----------------------------------|----------------|
/// | T0   | Exact SHA-256 hash match          | 0.0 or 1.0     |
/// | T1   | Jaccard word-overlap similarity    | 0.0 … 1.0      |
/// | T2+  | Jaccard fallback (same as T1)      | 0.0 … 1.0      |
pub fn compare(a: &str, b: &str, tier: Tier) -> f64 {
    match tier {
        Tier::T0 => exact_hash_match(a, b),
        Tier::T1 | Tier::T2 | Tier::T3 => jaccard_similarity(a, b),
    }
}

// ---------------------------------------------------------------------------
// HashOnlyProvider
// ---------------------------------------------------------------------------

/// Stateless provider for T0/T1 similarity comparison.
///
/// The [`SimilarityProvider`] trait method accepts `(agent_id, new_prompt)`
/// but the provider has **no storage** to look up previous prompts, so it
/// returns `Ok(None)` — signalling callers to fall back to their own
/// Jaccard / hash logic (e.g. via `DriftIndex` in `forge_drift`).
///
/// Callers who already hold **both** strings should use the free function
/// [`compare`] or the inherent [`HashOnlyProvider::compare_strings`]
/// method instead.
pub struct HashOnlyProvider;

impl HashOnlyProvider {
    pub fn new() -> Self {
        Self
    }

    /// Compare two strings directly using the given [`Tier`].
    ///
    /// This is a convenience wrapper around the free [`compare`] function.
    /// Use it when both the old and new prompts are immediately available.
    pub fn compare_strings(&self, a: &str, b: &str, tier: Tier) -> f64 {
        compare(a, b, tier)
    }
}

impl Default for HashOnlyProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SimilarityProvider for HashOnlyProvider {
    async fn compare(
        &self,
        _agent_id: &str,
        _new_prompt: &str,
    ) -> Result<Option<f64>, SimilarityError> {
        // No persistent storage — cannot look up the previous prompt.
        // Returns None so the caller falls back to its own T0/T1 path
        // (e.g. DriftIndex::is_exact_match / DriftIndex::jaccard).
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- SHA-256 exact match ---

    #[test]
    fn test_exact_hash_match_identical() {
        let s = "The quick brown fox jumps over the lazy dog";
        assert_eq!(exact_hash_match(s, s), 1.0);
    }

    #[test]
    fn test_exact_hash_match_different() {
        assert_eq!(exact_hash_match("hello world", "world hello"), 0.0);
    }

    #[test]
    fn test_exact_hash_match_empty() {
        assert_eq!(exact_hash_match("", ""), 1.0);
    }

    // --- Jaccard similarity ---

    #[test]
    fn test_jaccard_identical() {
        assert!((jaccard_similarity("hello world", "hello world") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_partial() {
        // A = {hello, world}, B = {hello, there}
        // |A ∩ B| = 1, |A ∪ B| = 3  =>  1/3 ≈ 0.333...
        let score = jaccard_similarity("hello world", "hello there");
        assert!((score - 1.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn test_jaccard_disjoint() {
        assert_eq!(jaccard_similarity("hello world", "foo bar"), 0.0);
    }

    #[test]
    fn test_jaccard_one_subset() {
        // A = {hello}, B = {hello, world}
        // |A ∩ B| = 1, |A ∪ B| = 2  =>  0.5
        let score = jaccard_similarity("hello", "hello world");
        assert!((score - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_both_empty() {
        assert_eq!(jaccard_similarity("", ""), 1.0);
    }

    #[test]
    fn test_jaccard_one_empty() {
        assert_eq!(jaccard_similarity("hello world", ""), 0.0);
    }

    // --- Public dispatcher ---

    #[test]
    fn test_compare_t0_identical() {
        assert_eq!(compare("same prompt", "same prompt", Tier::T0), 1.0);
    }

    #[test]
    fn test_compare_t0_different() {
        assert_eq!(compare("prompt a", "prompt b", Tier::T0), 0.0);
    }

    #[test]
    fn test_compare_t0_vs_t1() {
        // T0 sees these as different (different bytes → different hash)
        assert_eq!(compare("hello world", "HELLO WORLD", Tier::T0), 0.0);
        // T1 sees partial overlap: {hello, world} ∩ {HELLO, WORLD} = {}
        assert_eq!(
            compare("hello world", "HELLO WORLD", Tier::T1),
            jaccard_similarity("hello world", "HELLO WORLD")
        );
    }

    #[test]
    fn test_compare_t1_partial() {
        let score = compare("hello world", "hello there", Tier::T1);
        assert!((score - 1.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn test_compare_t2_falls_back_to_jaccard() {
        // T2+ should gracefully degrade to Jaccard
        let score = compare("hello world", "hello there", Tier::T2);
        assert!((score - 1.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn test_compare_strings_inherent() {
        let p = HashOnlyProvider::new();
        assert_eq!(p.compare_strings("same", "same", Tier::T0), 1.0);
        assert!((p.compare_strings("a b", "a c", Tier::T1) - 1.0 / 3.0).abs() < f64::EPSILON);
    }
}
