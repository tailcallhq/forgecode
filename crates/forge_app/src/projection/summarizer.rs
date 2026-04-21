use std::path::Path;

use forge_domain::{
    Compact, Context, ContextMessage, ContextSummary, MessageEntry, MessageId, PendingTurn, Role,
    Template, Transformer,
};

use super::{
    CompactionMethod, ProjectedEntry, Projection, ProjectionConfig, ProjectorInput, SummaryPayload,
};
use crate::TemplateEngine;
use crate::transformers::SummaryTransformer;

const SUMMARY_TEMPLATE: &str = "forge-partial-summary-frame.md";

/// Single forward scan over canonical. Flushes summary frames at valid
/// boundaries whenever a compact trigger fires against the assembled
/// request shape, then slides the summary list to the last N frames.
pub fn project(input: &ProjectorInput<'_>) -> anyhow::Result<Projection> {
    project_inner(
        input.canonical,
        input.pending,
        input.compact,
        input.config,
        input.cwd,
        input.max_prepended_summaries,
    )
}

fn project_inner(
    canonical: &Context,
    pending: &PendingTurn,
    compact: &Compact,
    config: &ProjectionConfig,
    cwd: &Path,
    max_prepended_summaries: usize,
) -> anyhow::Result<Projection> {
    // `on_turn_end` is once-per-projection, not per-step — armed iff
    // the tail of pending (= last msg of the assembled request) is a
    // user message.
    let on_turn_end_armed =
        compact.on_turn_end == Some(true) && pending_tail_is_user(pending);

    let mut buffer: Vec<MessageEntry> = Vec::new();
    let mut summaries: Vec<SummaryPayload> = Vec::new();

    let messages = &canonical.messages;
    for idx in 0..messages.len() {
        buffer.push(messages[idx].clone());

        // Triggers evaluate against the assembled request shape at this
        // step — old summaries destined to slide off are excluded,
        // pending is included — so the budget matches what the model
        // would see if the walk stopped here.
        if trigger_fires(
            &summaries,
            &buffer,
            pending,
            compact,
            config,
            max_prepended_summaries,
        ) && is_valid_flush_at_end(&buffer, messages.get(idx + 1))
        {
            flush_summary(&mut buffer, &mut summaries, cwd)?;
        }
    }

    // `on_turn_end` obligation: force one summary if armed and the walk
    // hasn't produced any. No valid cut = silent no-op (fallback rule
    // matches base's `find_sequence_preserving_last_n` returning None).
    if on_turn_end_armed
        && summaries.is_empty()
        && let Some(cut) = last_valid_cut(&buffer)
    {
        let to_summarize: Vec<MessageEntry> = buffer.drain(..=cut).collect();
        let payload = render_summary(&to_summarize, cwd)?;
        summaries.push(payload);
    }

    // Lossy true-sliding: older frames drop entirely once the cap is
    // hit; content not in the last N frames is gone.
    let skip = summaries.len().saturating_sub(max_prepended_summaries);
    let kept: Vec<SummaryPayload> = summaries.into_iter().skip(skip).collect();

    let mut entries: Vec<ProjectedEntry> = Vec::with_capacity(kept.len() + buffer.len());
    for payload in kept {
        entries.push(ProjectedEntry::Summary(payload));
    }
    for entry in buffer {
        entries.push(ProjectedEntry::Original(Box::new(entry)));
    }

    Ok(Projection { entries, directives: Vec::new() })
}

fn flush_summary(
    buffer: &mut Vec<MessageEntry>,
    summaries: &mut Vec<SummaryPayload>,
    cwd: &Path,
) -> anyhow::Result<()> {
    let drained: Vec<MessageEntry> = std::mem::take(buffer);
    let payload = render_summary(&drained, cwd)?;
    summaries.push(payload);
    Ok(())
}

fn render_summary(entries: &[MessageEntry], cwd: &Path) -> anyhow::Result<SummaryPayload> {
    let source_ids: Vec<MessageId> = entries.iter().map(|e| e.id).collect();
    let sequence_context = Context::default().messages(entries.to_vec());
    let summary = ContextSummary::from(&sequence_context);
    let summary = SummaryTransformer::new(cwd).transform(summary);
    let text = TemplateEngine::default().render(
        Template::<ContextSummary>::new(SUMMARY_TEMPLATE),
        &summary,
    )?;
    Ok(SummaryPayload { method: CompactionMethod::Template, source_ids, text })
}

/// Evaluates per-step triggers against
/// `[last N of summaries-so-far][buffer][pending]`. `on_turn_end` is
/// deliberately absent — its obligation is evaluated once per
/// projection, not on every walk step.
fn trigger_fires(
    summaries: &[SummaryPayload],
    buffer: &[MessageEntry],
    pending: &PendingTurn,
    compact: &Compact,
    config: &ProjectionConfig,
    cap: usize,
) -> bool {
    // Only the last N summaries-so-far count — frames destined to
    // slide off at the end must not inflate mid-walk trigger decisions.
    let skip = summaries.len().saturating_sub(cap);
    let kept_summaries = &summaries[skip..];

    // `token_threshold_percentage` is folded into
    // `effective_token_threshold` upstream, so one comparison covers
    // both knobs.
    let assembled_tokens = summaries_tokens(kept_summaries)
        + buffer
            .iter()
            .map(|e| e.token_count_approx())
            .sum::<usize>()
        + pending.token_count_approx();
    if assembled_tokens >= config.effective_token_threshold {
        return true;
    }

    if let Some(msg_threshold) = compact.message_threshold {
        let msg_count = kept_summaries.len() + buffer.len() + pending.iter_messages().count();
        if msg_count >= msg_threshold {
            return true;
        }
    }

    // Rendered summary frames are inserted as user messages, so each
    // one counts as a turn — matches base's `should_compact_due_to_turns`.
    if let Some(turn_threshold) = compact.turn_threshold {
        let user_count = kept_summaries.len()
            + buffer
                .iter()
                .filter(|e| is_user_text(e))
                .count()
            + pending
                .iter_messages()
                .filter(|e| is_user_text(e))
                .count();
        if user_count >= turn_threshold {
            return true;
        }
    }

    false
}

fn summaries_tokens(summaries: &[SummaryPayload]) -> usize {
    summaries
        .iter()
        .map(|s| s.text.chars().count().div_ceil(4))
        .sum()
}

fn is_user_text(e: &MessageEntry) -> bool {
    matches!(&e.message, ContextMessage::Text(t) if t.role == Role::User)
}

fn is_toolcall(e: &MessageEntry) -> bool {
    matches!(
        &e.message,
        ContextMessage::Text(t)
            if t.role == Role::Assistant
            && t.tool_calls.as_ref().is_some_and(|c| !c.is_empty())
    )
}

fn is_toolcall_result(e: &MessageEntry) -> bool {
    matches!(&e.message, ContextMessage::Tool(_))
}

fn is_assistant(e: &MessageEntry) -> bool {
    matches!(&e.message, ContextMessage::Text(t) if t.role == Role::Assistant)
}

/// Enforces the flush-boundary rules from REQUIREMENTS:
/// - hard: never split a `tool_call`/`tool_result` pair or a parallel
///   `tool_result` group;
/// - soft: the next buffer should start with an assistant. During the
///   forward scan this is treated as hard because the walker can
///   always keep appending; leftover-at-EOS is the fallback path.
fn is_valid_flush_at_end(buffer: &[MessageEntry], next: Option<&MessageEntry>) -> bool {
    let Some(last) = buffer.last() else {
        return false;
    };
    if is_toolcall(last) {
        return false;
    }
    if is_toolcall_result(last) && next.is_some_and(is_toolcall_result) {
        return false;
    }
    // End-of-scan (`next == None`) is valid — there's no future
    // assistant to wait for, so we flush the hot trigger now.
    match next {
        Some(n) => is_assistant(n),
        None => true,
    }
}

/// Latest index where `buffer[..=i]` ends at a valid flush boundary.
/// Used only by the `on_turn_end` obligation. Prefers cuts whose new
/// buffer starts with an assistant; if none satisfy the soft rule,
/// falls back to atomicity-only (REQUIREMENTS: "where possible").
fn last_valid_cut(buffer: &[MessageEntry]) -> Option<usize> {
    let strict = (0..buffer.len())
        .rev()
        .find(|&i| is_valid_cut_at(buffer, i, true));
    strict.or_else(|| {
        (0..buffer.len())
            .rev()
            .find(|&i| is_valid_cut_at(buffer, i, false))
    })
}

fn is_valid_cut_at(buffer: &[MessageEntry], i: usize, prefer_assistant_next: bool) -> bool {
    if is_toolcall(&buffer[i]) {
        return false;
    }
    if is_toolcall_result(&buffer[i])
        && i + 1 < buffer.len()
        && is_toolcall_result(&buffer[i + 1])
    {
        return false;
    }
    if prefer_assistant_next {
        match buffer.get(i + 1) {
            None => true,
            Some(next) => is_assistant(next),
        }
    } else {
        true
    }
}

fn pending_tail_is_user(pending: &PendingTurn) -> bool {
    pending
        .iter_messages()
        .last()
        .is_some_and(is_user_text)
}

#[cfg(test)]
mod tests {
    use forge_domain::{
        ContextMessage, MessageEntry, Role, TextMessage, ToolCallFull, ToolCallId, ToolName,
        ToolOutput, ToolResult,
    };
    use pretty_assertions::assert_eq;

    use super::*;

    fn cwd() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp")
    }

    fn cfg(threshold: usize) -> ProjectionConfig {
        ProjectionConfig { effective_token_threshold: threshold }
    }

    fn user(text: &str) -> MessageEntry {
        MessageEntry::from(ContextMessage::Text(TextMessage::new(Role::User, text)))
    }

    fn assistant(text: &str) -> MessageEntry {
        MessageEntry::from(ContextMessage::Text(TextMessage::new(Role::Assistant, text)))
    }

    fn assistant_with_tool(text: &str, call_id: &str) -> MessageEntry {
        MessageEntry::from(ContextMessage::Text(
            TextMessage::new(Role::Assistant, text)
                .tool_calls(vec![ToolCallFull::new(ToolName::new("read")).call_id(call_id)]),
        ))
    }

    fn tool_result(call_id: &str) -> MessageEntry {
        MessageEntry::from(ContextMessage::Tool(ToolResult {
            name: ToolName::new("read"),
            call_id: Some(ToolCallId::new(call_id)),
            output: ToolOutput::text("ok"),
        }))
    }

    fn context(msgs: Vec<MessageEntry>) -> Context {
        Context::default().messages(msgs)
    }

    fn compact_with_msg_threshold(n: usize) -> Compact {
        let mut c = Compact::new();
        c.message_threshold = Some(n);
        c
    }

    fn run(
        ctx: &Context,
        pending: &PendingTurn,
        compact: &Compact,
        config: &ProjectionConfig,
        cap: usize,
    ) -> anyhow::Result<Projection> {
        let cwd_buf = cwd();
        let input = ProjectorInput {
            canonical: ctx,
            pending,
            compact,
            config,
            cwd: &cwd_buf,
            max_prepended_summaries: cap,
        };
        project(&input)
    }

    /// Zero summaries when no trigger is configured — nothing to fire on.
    #[test]
    fn test_no_trigger_passes_through() {
        let ctx = context(vec![user("q1"), assistant("a1"), user("q2")]);
        let pending = PendingTurn::default();
        let compact = Compact::new();

        let projection = run(&ctx, &pending, &compact, &cfg(usize::MAX), 2).unwrap();

        assert_eq!(projection.entries.len(), 3);
        assert!(
            projection
                .entries
                .iter()
                .all(|e| matches!(e, ProjectedEntry::Original(_)))
        );
    }

    /// Post-flush assembled size (1 summary + leftover) stays below the
    /// threshold, so no second flush fires — guards against runaway
    /// re-triggering once a summary enters the assembled count.
    #[test]
    fn test_message_threshold_fires_at_valid_boundary() {
        let ctx = context(vec![user("q1"), assistant("a1"), user("q2"), assistant("a2")]);
        let pending = PendingTurn::default();
        let compact = compact_with_msg_threshold(3);

        let projection =
            run(&ctx, &pending, &compact, &cfg(usize::MAX), 2).unwrap();

        let summaries: Vec<_> = projection
            .entries
            .iter()
            .filter(|e| matches!(e, ProjectedEntry::Summary(_)))
            .collect();
        assert_eq!(summaries.len(), 1, "expected one summary frame");

        let originals: Vec<_> = projection
            .entries
            .iter()
            .filter(|e| matches!(e, ProjectedEntry::Original(_)))
            .collect();
        assert_eq!(originals.len(), 1, "expected a single trailing message in leftover buffer");
    }

    /// Guards tool-pair atomicity: a trigger that fires mid-pair must
    /// defer to the next valid boundary. Dangling tool halves land the
    /// request in a 400 at the provider.
    #[test]
    fn test_tool_call_and_result_flush_together() {
        let ctx = context(vec![
            user("q1"),
            assistant_with_tool("calling", "c1"),
            tool_result("c1"),
            user("q2"),
        ]);
        let pending = PendingTurn::default();
        // Threshold = 2 would fire after the tool_call (buffer size 2);
        // algorithm must defer until after the tool_result lands.
        let compact = compact_with_msg_threshold(2);

        let projection =
            run(&ctx, &pending, &compact, &cfg(usize::MAX), 2).unwrap();

        // The leftover buffer must not contain a bare tool_call or bare
        // tool_result; they either both survive or both get folded into the
        // summary together.
        let originals: Vec<&MessageEntry> = projection
            .entries
            .iter()
            .filter_map(|e| match e {
                ProjectedEntry::Original(m) => Some(m.as_ref()),
                _ => None,
            })
            .collect();
        let has_orphan_call = originals.iter().any(|e| is_toolcall(e));
        let has_orphan_result = originals.iter().any(|e| is_toolcall_result(e));
        assert_eq!(
            has_orphan_call, has_orphan_result,
            "tool_call and tool_result must either both fold or both stay"
        );
    }

    /// Cap bounds the summary-prefix size regardless of how aggressive
    /// the trigger is — prevents unbounded growth from cascading flushes.
    #[test]
    fn test_sliding_cap_drops_oldest_summaries() {
        let ctx = context(vec![
            user("q1"),
            assistant("a1"),
            user("q2"),
            assistant("a2"),
            user("q3"),
            assistant("a3"),
            user("q4"),
        ]);
        let pending = PendingTurn::default();
        let compact = compact_with_msg_threshold(2);

        let projection = run(&ctx, &pending, &compact, &cfg(usize::MAX), 2).unwrap();

        let summaries: Vec<_> = projection
            .entries
            .iter()
            .filter(|e| matches!(e, ProjectedEntry::Summary(_)))
            .collect();
        assert!(
            summaries.len() <= 2,
            "sliding cap must keep at most 2 summaries, got {}",
            summaries.len()
        );
    }

    /// Mirrors base's `start-at-first-assistant` rule from within the
    /// forward scan: a trigger firing on `[user, user]` must defer the
    /// flush until the next canonical message is an assistant, so the
    /// new buffer starts at an assistant message.
    #[test]
    fn test_flush_defers_until_next_is_assistant() {
        let ctx = context(vec![
            user("q1"),
            user("q2"),
            assistant("a1"),
            user("q3"),
            assistant("a2"),
        ]);
        let pending = PendingTurn::default();
        let compact = compact_with_msg_threshold(2);

        let projection = run(&ctx, &pending, &compact, &cfg(usize::MAX), 2).unwrap();

        // First flushable boundary is at index 2 (before the 'a1'
        // assistant) — the summary folds `[q1, q2]`, not `[q1]` alone.
        let summary = projection
            .entries
            .iter()
            .find_map(|e| match e {
                ProjectedEntry::Summary(s) => Some(s),
                _ => None,
            })
            .expect("expected a summary frame");
        assert_eq!(
            summary.source_ids.len(),
            2,
            "summary must span both leading user messages, got {}",
            summary.source_ids.len()
        );
    }

    /// `on_turn_end` alone — with every budget trigger dormant — still
    /// forces one summary because the obligation is independent of
    /// threshold checks.
    #[test]
    fn test_on_turn_end_forces_summary_when_armed() {
        let ctx = context(vec![user("q1"), assistant("a1"), user("q2"), assistant("a2")]);
        let mut pending = PendingTurn::default();
        pending.push_user_input(ContextMessage::Text(TextMessage::new(Role::User, "q3")));

        let mut compact = Compact::new();
        compact.on_turn_end = Some(true);

        let projection = run(&ctx, &pending, &compact, &cfg(usize::MAX), 2).unwrap();

        let summaries: Vec<_> = projection
            .entries
            .iter()
            .filter(|e| matches!(e, ProjectedEntry::Summary(_)))
            .collect();
        assert_eq!(summaries.len(), 1, "on_turn_end must produce at least one summary");
    }

    /// An unsatisfiable flush is a silent no-op, not a hard error —
    /// matches base's `find_sequence_preserving_last_n` returning None.
    #[test]
    fn test_no_valid_boundary_falls_back_to_pass_through() {
        let ctx = context(vec![user("q1"), user("q2"), user("q3")]);
        let mut pending = PendingTurn::default();
        pending.push_user_input(ContextMessage::Text(TextMessage::new(Role::User, "q4")));
        let mut compact = Compact::new();
        compact.on_turn_end = Some(true);
        compact.message_threshold = Some(1);

        let projection = run(&ctx, &pending, &compact, &cfg(0), 2).unwrap();

        // Degenerate all-user canonical: summarising it is meaningless
        // but the algorithm must not panic, and output must be coherent.
        assert!(!projection.entries.is_empty());
    }

    /// Summary text is byte-stable across repeated projections so the
    /// request hash stays the same — a prerequisite for any future
    /// sidecar memoisation or response caching.
    #[test]
    fn test_projection_is_deterministic() {
        let ctx = context(vec![user("q1"), assistant("a1"), user("q2"), assistant("a2")]);
        let pending = PendingTurn::default();
        let compact = compact_with_msg_threshold(2);

        let first = run(&ctx, &pending, &compact, &cfg(usize::MAX), 2).unwrap();
        let second = run(&ctx, &pending, &compact, &cfg(usize::MAX), 2).unwrap();

        let extract_summary = |p: &Projection| -> Option<String> {
            p.entries.iter().find_map(|e| match e {
                ProjectedEntry::Summary(SummaryPayload { text, .. }) => Some(text.clone()),
                _ => None,
            })
        };

        assert_eq!(extract_summary(&first), extract_summary(&second));
    }
}
