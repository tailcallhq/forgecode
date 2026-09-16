//! Deduplication logic for code search results across multiple queries.
//!
//! When performing batch semantic searches, the same code node may appear in
//! multiple queries with different scores. This module provides functionality
//! to deduplicate results, keeping each node only in the query where it has
//! the best score.

use std::cmp::Ordering;
use std::collections::HashMap;

use forge_domain::{Node, NodeId};

/// Tracks the best score for a node across multiple queries.
///
/// Implements `Ord` to enable comparison based on score quality.
/// Priority: relevance (higher is better) → distance (lower is better) →
/// similarity (higher is better) → query index (lower is better, tie-breaker).
#[derive(Debug, Clone, PartialEq)]
struct Score {
    query_idx: usize,
    relevance: Option<f32>,
    distance: Option<f32>,
}

impl Score {
    /// Creates a new `BestScore` from a query index and search result.
    fn new(query_idx: usize, result: &Node) -> Self {
        Self {
            query_idx,
            relevance: result.relevance,
            distance: result.distance,
        }
    }
}

impl Eq for Score {}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Score {
    fn cmp(&self, other: &Self) -> Ordering {
        /// Helper to compare two `Option<f32>` values (higher is better).
        ///
        /// # Returns
        /// - `Some(Ordering)` if comparison is decisive
        /// - `None` to continue to next comparison
        fn compare(a: Option<f32>, b: Option<f32>) -> Option<Ordering> {
            match (a, b) {
                (Some(x), Some(y)) => match x.partial_cmp(&y)? {
                    Ordering::Equal => None, // Continue to next comparison
                    ord => Some(ord),
                },
                (Some(_), None) => Some(Ordering::Greater), // Having a value is better than None
                (None, Some(_)) => Some(Ordering::Less),    // None is worse than having a value
                (None, None) => None,                       // Continue to next comparison
            }
        }

        // Compare in priority order: relevance → distance → similarity → query
        // index
        compare(self.relevance, other.relevance) // Higher relevance is better
            .or_else(|| compare(other.distance, self.distance)) // Lower distance is better (flipped)
            .unwrap_or_else(|| self.query_idx.cmp(&other.query_idx).reverse()) // Lower query index wins (first query wins)
    }
}

/// Deduplicates code search results across multiple queries.
///
/// Each node appears only once across all query results, kept in the query
/// where it has the highest score according to the `BestScore` ordering.
///
/// # Arguments
/// * `results` - Vector of search results per query (will be modified in place)
///
/// # Errors
/// Returns an error if node IDs cannot be extracted from results.
pub fn deduplicate_results(results: &mut [Vec<Node>]) {
    // Track best score for each node_id across all queries
    let mut best_scores: HashMap<NodeId, Score> = HashMap::new();

    // First pass: find which query has the best score for each node
    for (query_idx, query_results) in results.iter().enumerate() {
        for result in query_results {
            let current_score = Score::new(query_idx, result);
            match best_scores.entry(result.node_id.clone()) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if current_score > *entry.get() {
                        entry.insert(current_score);
                    }
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(current_score);
                }
            }
        }
    }

    // Second pass: remove duplicates, keeping only in the query with best score
    for (query_idx, query_results) in results.iter_mut().enumerate() {
        query_results.retain(|result| {
            best_scores
                .get(&result.node_id)
                .is_none_or(|best| best.query_idx == query_idx)
        });
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Node, NodeData};
    use pretty_assertions::assert_eq;

    use super::*;

    /// Test fixture for creating a minimal `CodeSearchResult`.
    fn result(node_id: &str) -> Node {
        Node {
            node_id: node_id.into(),
            node: NodeData::FileChunk(forge_domain::FileChunk {
                file_path: "test.rs".into(),
                content: "test".into(),
                start_line: 1,
                end_line: 1,
            }),
            relevance: None,
            distance: None,
        }
    }

    #[test]
    fn test_best_score_ordering_by_relevance() {
        let score1 = Score::new(0, &result("node_a").relevance(0.9));
        let score2 = Score::new(1, &result("node_a").relevance(0.8));

        assert!(score1 > score2);
    }

    #[test]
    fn test_best_score_ordering_by_distance_when_relevance_equal() {
        let score1 = Score::new(0, &result("node_a").relevance(0.9).distance(0.1));
        let score2 = Score::new(1, &result("node_a").relevance(0.9).distance(0.2));

        assert!(score1 > score2);
    }

    #[test]
    fn test_best_score_ordering_by_similarity_when_relevance_distance_equal() {
        let score1 = Score::new(0, &result("node_a").relevance(0.9).distance(0.1));
        let score2 = Score::new(1, &result("node_a").relevance(0.9).distance(0.1));

        assert!(score1 > score2);
    }

    #[test]
    fn test_best_score_ordering_by_query_idx_when_all_equal() {
        let score1 = Score::new(0, &result("node_a").relevance(0.9).distance(0.1));
        let score2 = Score::new(1, &result("node_a").relevance(0.9).distance(0.1));

        assert!(score1 > score2); // Lower query index wins
    }

    #[test]
    fn test_best_score_some_value_better_than_none() {
        let score1 = Score::new(0, &result("node_a").relevance(0.5));
        let score2 = Score::new(1, &result("node_a"));

        assert!(score1 > score2);
    }

    #[test]
    fn test_deduplicate_results_keeps_highest_relevance() {
        let mut actual = vec![
            vec![
                result("node_a").relevance(0.8).distance(0.2),
                result("node_b").relevance(0.7).distance(0.3),
            ],
            vec![
                result("node_a").relevance(0.9).distance(0.1),
                result("node_c").relevance(0.6).distance(0.4),
            ],
        ];

        deduplicate_results(&mut actual);

        let expected = vec![
            vec![result("node_b").relevance(0.7).distance(0.3)],
            vec![
                result("node_a").relevance(0.9).distance(0.1),
                result("node_c").relevance(0.6).distance(0.4),
            ],
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_deduplicate_multiple_duplicates() {
        let mut actual = vec![
            vec![
                result("node_a").relevance(0.8).distance(0.2),
                result("node_b").relevance(0.7).distance(0.3),
                result("node_c").relevance(0.6).distance(0.4),
            ],
            vec![
                result("node_a").relevance(0.9).distance(0.1),
                result("node_b").relevance(0.5).distance(0.5),
                result("node_d").relevance(0.95).distance(0.05),
            ],
        ];

        deduplicate_results(&mut actual);

        let expected = vec![
            vec![
                result("node_b").relevance(0.7).distance(0.3),
                result("node_c").relevance(0.6).distance(0.4),
            ],
            vec![
                result("node_a").relevance(0.9).distance(0.1),
                result("node_d").relevance(0.95).distance(0.05),
            ],
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_deduplicate_equal_relevance_uses_distance_tiebreaker() {
        let mut actual = vec![
            vec![
                result("node_a").relevance(0.9).distance(0.2),
                result("node_b").relevance(0.8).distance(0.2),
            ],
            vec![
                result("node_a").relevance(0.9).distance(0.1),
                result("node_c").relevance(0.7).distance(0.3),
            ],
        ];

        deduplicate_results(&mut actual);

        let expected = vec![
            vec![result("node_b").relevance(0.8).distance(0.2)],
            vec![
                result("node_a").relevance(0.9).distance(0.1),
                result("node_c").relevance(0.7).distance(0.3),
            ],
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_deduplicate_across_three_queries() {
        let mut actual = vec![
            vec![
                result("node_a").relevance(0.85).distance(0.15),
                result("node_b").relevance(0.75).distance(0.25),
                result("node_e").relevance(0.65).distance(0.35),
            ],
            vec![
                result("node_a").relevance(0.90).distance(0.10),
                result("node_c").relevance(0.80).distance(0.20),
                result("node_d").relevance(0.70).distance(0.30),
            ],
            vec![
                result("node_a").relevance(0.88).distance(0.12),
                result("node_b").relevance(0.78).distance(0.22),
                result("node_d").relevance(0.72).distance(0.28),
            ],
        ];

        deduplicate_results(&mut actual);

        let expected = vec![
            vec![result("node_e").relevance(0.65).distance(0.35)],
            vec![
                result("node_a").relevance(0.90).distance(0.10),
                result("node_c").relevance(0.80).distance(0.20),
            ],
            vec![
                result("node_b").relevance(0.78).distance(0.22),
                result("node_d").relevance(0.72).distance(0.28),
            ],
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_deduplicate_all_scores_equal_first_query_wins() {
        let mut actual = vec![
            vec![result("node_a").relevance(0.8).distance(0.2)],
            vec![result("node_a").relevance(0.8).distance(0.2)],
        ];

        deduplicate_results(&mut actual);

        let expected = vec![vec![result("node_a").relevance(0.8).distance(0.2)], vec![]];

        assert_eq!(actual, expected);
    }
}
