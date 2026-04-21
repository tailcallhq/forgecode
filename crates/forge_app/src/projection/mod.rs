use std::path::Path;

use forge_domain::{Compact, Context, MessageEntry, MessageId, PendingTurn};

use crate::Error;

mod message_entry_adapter;
mod summarizer;

pub use message_entry_adapter::CompactableEntry;

/// A canonical message preserved verbatim, or a summary that replaces a
/// span of canonical messages.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectedEntry {
    /// Boxed to keep the enum size from being dominated by `MessageEntry`.
    Original(Box<MessageEntry>),
    Summary(SummaryPayload),
}

/// Summary content that replaces a span of canonical messages.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryPayload {
    pub method: CompactionMethod,
    /// Canonical ids folded into this summary, in canonical order.
    pub source_ids: Vec<MessageId>,
    pub text: String,
}

/// How a summary was produced.
#[derive(Debug, Clone, PartialEq)]
pub enum CompactionMethod {
    /// Deterministic template render; no LLM call.
    Template,
}

/// Request-side directive slot. Empty today — reserved so adding
/// directives doesn't change the projector → request-builder signature.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestDirective {}

/// A request-time projection of canonical. Fully reconstructed per
/// request; not persisted.
#[derive(Debug, Clone, PartialEq)]
pub struct Projection {
    pub entries: Vec<ProjectedEntry>,
    pub directives: Vec<RequestDirective>,
}

/// `Passthrough` forwards canonical unchanged; `Summarize` runs the
/// forward-scan template summarizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Passthrough,
    Summarize,
}

/// Resolved thresholds for tier selection. Populated from `Compact`
/// after the agent's preprocessing has derived the token threshold.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionConfig {
    pub effective_token_threshold: usize,
}

impl ProjectionConfig {
    /// Dispatches to `Summarize` once the combined canonical+pending
    /// token count reaches the configured threshold.
    pub fn select_tier(&self, request_tokens: usize) -> Tier {
        if request_tokens >= self.effective_token_threshold {
            Tier::Summarize
        } else {
            Tier::Passthrough
        }
    }
}

impl TryFrom<&Compact> for ProjectionConfig {
    type Error = Error;

    fn try_from(compact: &Compact) -> Result<Self, Self::Error> {
        let effective_token_threshold = compact
            .token_threshold
            .ok_or(Error::ProjectionConfigNotReady)?;
        Ok(Self { effective_token_threshold })
    }
}

/// Bundle of inputs a tier's `project` function consumes. Packaged so
/// new tiers (e.g. an LLM summariser) can be added without churn on
/// every call site.
pub struct ProjectorInput<'a> {
    pub canonical: &'a Context,
    pub pending: &'a PendingTurn,
    pub compact: &'a Compact,
    pub config: &'a ProjectionConfig,
    pub cwd: &'a Path,
    pub max_prepended_summaries: usize,
}

/// Dispatch point for projection tiers. New tiers (e.g. an LLM
/// summariser) register here without the orchestrator needing to learn
/// their shape. `async` today so a future tier with an I/O dispatch can
/// slot in without changing this signature.
pub struct Projector;

impl Projector {
    pub async fn project(tier: Tier, input: &ProjectorInput<'_>) -> anyhow::Result<Projection> {
        match tier {
            Tier::Passthrough => Ok(passthrough(input.canonical)),
            Tier::Summarize => summarizer::project(input),
        }
    }
}

fn passthrough(context: &Context) -> Projection {
    let entries = context
        .messages
        .iter()
        .cloned()
        .map(|entry| ProjectedEntry::Original(Box::new(entry)))
        .collect();
    Projection { entries, directives: Vec::new() }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn config(threshold: usize) -> ProjectionConfig {
        ProjectionConfig { effective_token_threshold: threshold }
    }

    /// Below threshold selects `Passthrough`; at or above selects `Summarize`.
    #[test]
    fn test_select_tier_bands() {
        let cfg = config(100);
        assert_eq!(cfg.select_tier(0), Tier::Passthrough);
        assert_eq!(cfg.select_tier(99), Tier::Passthrough);
        assert_eq!(cfg.select_tier(100), Tier::Summarize);
        assert_eq!(cfg.select_tier(10_000), Tier::Summarize);
    }

    /// `ProjectionConfig::try_from` refuses to build with an unpopulated
    /// token threshold so callers don't silently dispatch `Tier0`.
    #[test]
    fn test_projection_config_requires_derived_threshold() {
        let compact = Compact::new();
        let err = ProjectionConfig::try_from(&compact).unwrap_err();
        assert!(matches!(err, Error::ProjectionConfigNotReady));
    }

    /// A populated threshold reads back verbatim.
    #[test]
    fn test_projection_config_reads_derived_value() {
        let mut compact = Compact::new();
        compact.token_threshold = Some(89_600);

        let cfg = ProjectionConfig::try_from(&compact).unwrap();

        assert_eq!(cfg.effective_token_threshold, 89_600);
    }

    /// Keeps `SummaryPayload` from being stripped as dead code during
    /// refactors that temporarily disable the projector.
    #[test]
    fn test_summary_payload_constructs_with_source_ids() {
        let payload = SummaryPayload {
            method: CompactionMethod::Template,
            source_ids: vec![MessageId::new()],
            text: "summary".to_string(),
        };
        assert_eq!(payload.source_ids.len(), 1);
        assert_eq!(payload.text, "summary");
    }

    #[allow(dead_code)]
    fn _directive_match(directive: RequestDirective) -> ! {
        match directive {}
    }
}
