//! AI-driven pruning of conversation context.
//!
//! Pruning removes low-value messages to stay within a token budget,
//! using importance scoring to decide what to keep vs discard.
//!
//! This is separate from compression: compression reduces message size,
//! while pruning removes entire messages.

use forge_domain::{Compact, Context, Role};

/// Report of what was pruned.
#[derive(Debug, Clone, Default)]
pub struct PruneReport {
    /// Indices of pruned messages (in original order).
    pub pruned: Vec<usize>,
    /// Tokens saved by pruning.
    pub tokens_saved: usize,
    /// Remaining token count after pruning.
    pub remaining_tokens: usize,
    /// Whether pruning was triggered at all.
    pub did_prune: bool,
}

/// Prune context to fit within the token budget.
///
/// Strategy:
/// 1. Compute available headroom: current_tokens - budget_tokens.
/// 2. Score each message by importance (heuristic).
/// 3. Remove lowest-scoring messages, from the middle outward, until within
///    budget or out of removable messages.
/// 4. Always preserve: system messages, first user message, last assistant
///    message.
pub fn prune(ctx: &Context, config: &Compact) -> (Context, PruneReport) {
    let mut report = PruneReport::default();
    let current_tokens = ctx.token_count_approx();
    let budget = config.token_threshold.unwrap_or(80_000);

    if current_tokens <= budget {
        report.remaining_tokens = current_tokens;
        return (ctx.clone(), report);
    }

    let mut pruned = Vec::new();

    // Score each message, preserving boundaries
    let scores: Vec<f64> = ctx
        .messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if i == 0 || i == ctx.messages.len() - 1 || m.has_role(Role::System) {
                1.0 // always preserve
            } else if m.is_droppable() {
                0.2 // easy to drop
            } else if m.has_tool_call() || m.has_tool_result() {
                0.8 // important
            } else if m.has_reasoning_details() {
                0.6 // moderately important
            } else {
                0.4 // default
            }
        })
        .collect();

    // Collect indices, sorted by score ascending (lowest first)
    let mut indices: Vec<(usize, f64)> = scores.into_iter().enumerate().collect();
    indices.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut remaining = ctx.clone();
    let prune_threshold = config.prune_threshold;

    for (idx, _score) in indices {
        if remaining.token_count_approx() <= budget {
            break;
        }
        if pruned.len() >= prune_threshold {
            break;
        }
        // Don't prune boundaries
        if idx == 0 || idx == remaining.messages.len().saturating_sub(1) {
            continue;
        }
        if remaining
            .messages
            .get(idx)
            .is_some_and(|message| message.has_role(Role::System))
        {
            continue;
        }
        remaining.messages.remove(idx);
        pruned.push(idx);
    }

    let tokens_after = remaining.token_count_approx();

    report.pruned = pruned;
    report.tokens_saved = current_tokens.saturating_sub(tokens_after);
    report.remaining_tokens = tokens_after;
    report.did_prune = !report.pruned.is_empty();

    (remaining, report)
}
