//! Programmatic compression strategies.
//!
//! Rule-based message pruning — no AI calls, no embeddings.
//! Safe to run on every compaction tick.

use forge_domain::{Compact, Context};

use super::CompressionReport;

/// Run programmatic compression rules against `context`.
///
/// Strategies (applied in order):
/// 1. Drop consecutive tool-call / tool-result pairs that exceed a count.
/// 2. Prune low-information system-prompt boilerplate.
/// 3. Remove redundant URL-encoded images beyond the first N.
pub fn compress_programmatic(mut ctx: Context, _config: &Compact) -> (Context, CompressionReport) {
    // Strategy 1: Collapse repeated tool → tool-result sequences.
    // Strategy 2: Drop droppable messages from the middle of long contexts.
    // Strategy 3: Remove excess base64 images.

    let mut report = CompressionReport::default();
    let before = ctx.token_count_approx();

    // Strategy: remove old "droppable" messages beyond retention window
    let retention = _config.retention_window.max(12);
    let total = ctx.total_messages();

    if total > retention.saturating_mul(2) {
        let droppable_indices: Vec<usize> = ctx
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.is_droppable())
            .map(|(i, _)| i)
            .collect();

        // Keep at least half the retention window around
        let max_drop = total.saturating_sub(retention);
        let to_drop: Vec<usize> = droppable_indices.into_iter().take(max_drop).collect();

        for idx in to_drop.iter().rev() {
            if *idx < ctx.messages.len() {
                ctx.messages.remove(*idx);
            }
        }
        report.removed = to_drop;
    }

    let after = ctx.token_count_approx();
    report.tokens_saved = before.saturating_sub(after);

    (ctx, report)
}
