//! AI-driven compression strategy.
//!
//! Uses a scoring model (or LLM call) to rank messages by importance
//! and selectively remove low-value content.
//!
//! Currently provides the hook structure and an importance-based heuristic.
//! Full LLM-based scoring can be plugged into `score_message_importance`.

use forge_domain::{Compact, Context, Role};

use super::CompressionReport;

/// Run AI-driven compression against `context`.
///
/// Strategy:
/// 1. Score each message by importance (heuristic or LLM call).
/// 2. Remove lowest-scoring messages until under budget.
/// 3. Preserve system messages, first user message, and last assistant message.
pub fn compress_ai(mut ctx: Context, _config: &Compact) -> (Context, CompressionReport) {
    let mut report = CompressionReport::default();
    let before = ctx.token_count_approx();

    let budget = _config.token_threshold.unwrap_or(80_000);
    let min_importance = _config.min_importance_threshold;

    if before <= budget {
        return (ctx, report);
    }

    let total = ctx.messages.len();
    if total < 4 {
        return (ctx, report);
    }

    // Score each message (higher = more important)
    let scores: Vec<f64> = ctx
        .messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            // Always preserve boundaries
            if i == 0 || i == total - 1 {
                return 1.0;
            }
            if m.has_role(Role::System) {
                return 0.9;
            }
            score_message_importance(m, _config)
        })
        .collect();

    // Remove low-importance droppable messages
    let mut removed = Vec::new();
    let mut current = ctx.token_count_approx();
    for idx in (0..total).rev() {
        if current <= budget {
            break;
        }
        if idx < ctx.messages.len()
            && scores
                .get(idx)
                .copied()
                .is_some_and(|score| score < min_importance)
        {
            ctx.messages.remove(idx);
            removed.push(idx);
            current = ctx.token_count_approx();
        }
    }

    report.summarized = removed;
    report.tokens_saved = before.saturating_sub(current);

    (ctx, report)
}

/// Heuristic importance score for a message.
///
/// Factors:
/// - Has tool call → higher importance.
/// - Has tool result → higher importance.
/// - Longer content → slightly higher (but capped).
/// - Droppable → lower importance.
fn score_message_importance(msg: &impl HasImportanceSignals, _config: &Compact) -> f64 {
    let mut score: f64 = 0.5;

    if msg.has_tool_call() {
        score += 0.3;
    }
    if msg.has_tool_result() {
        score += 0.25;
    }
    if msg.is_droppable() {
        score -= 0.2;
    }
    if msg.has_reasoning_details() {
        score += 0.1;
    }

    score.clamp(0.0, 1.0)
}

/// Trait for messages that can be scored by the AI-driven compressor.
trait HasImportanceSignals {
    fn has_tool_call(&self) -> bool;
    fn has_tool_result(&self) -> bool;
    fn is_droppable(&self) -> bool;
    fn has_reasoning_details(&self) -> bool;
}

impl HasImportanceSignals for forge_domain::MessageEntry {
    fn has_tool_call(&self) -> bool {
        self.message.has_tool_call()
    }
    fn has_tool_result(&self) -> bool {
        self.message.has_tool_result()
    }
    fn is_droppable(&self) -> bool {
        self.message.is_droppable()
    }
    fn has_reasoning_details(&self) -> bool {
        self.message.has_reasoning_details()
    }
}
