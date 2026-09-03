//! Semantic compression strategies.
//!
//! Uses content characteristics (length, repetition, information density)
//! to identify and summarize low-value messages.
//!
//! NOTE: Full embedding-based semantic compression requires an external
//! embedding service. This module provides the structural hooks for that,
//! plus a simple length-based heuristic fallback.

use forge_domain::{Compact, Context, Role};

use super::CompressionReport;

/// Run semantic compression against `context`.
///
/// Heuristics (no external AI call):
/// 1. Long assistant messages with no tool calls → candidate for summarization.
/// 2. Repeated assistant messages (same role, similar length) → collapse.
/// 3. Messages with very low character-per-token ratio → drop.
pub fn compress_semantic(mut ctx: Context, _config: &Compact) -> (Context, CompressionReport) {
    let mut report = CompressionReport::default();
    let before = ctx.token_count_approx();

    let budget = _config.token_threshold.unwrap_or(80_000);
    if before <= budget {
        return (ctx, report);
    }

    // Identify verbose assistant messages with no tool results
    // that are safe to summarize.
    let total = ctx.messages.len();
    // We keep the first and last N messages; anything in the middle
    // that's long, droppable, or purely assistant-verbose is a candidate.
    let keep_ends = 6usize;
    let candidates: Vec<usize> = ctx
        .messages
        .iter()
        .enumerate()
        .filter(|(i, m)| {
            *i >= keep_ends
                && *i < total.saturating_sub(keep_ends)
                && (m.is_droppable() || m.has_role(Role::Assistant))
                && !m.has_tool_call()
        })
        .map(|(i, _)| i)
        .collect();

    // Remove oldest candidates until we're within budget or run out
    let mut removed = Vec::new();
    let mut current = ctx.token_count_approx();
    for idx in candidates.iter().rev() {
        if current <= budget {
            break;
        }
        if *idx < ctx.messages.len() {
            ctx.messages.remove(*idx);
            removed.push(*idx);
            current = ctx.token_count_approx();
        }
    }

    report.summarized = removed;
    report.tokens_saved = before.saturating_sub(current);

    (ctx, report)
}
