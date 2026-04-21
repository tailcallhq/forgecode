use forge_domain::{Compact, Context, MessageEntry, MessageId};

use crate::Error;

/// A single entry in a projection: either a canonical message preserved
/// verbatim, or a summary that replaces a span of canonical messages.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectedEntry {
    /// Boxed so the enum size is not dominated by `MessageEntry`.
    Original(Box<MessageEntry>),
    Summary(SummaryPayload),
}

/// Summary content that replaces a span of canonical messages in a
/// projected sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryPayload {
    /// How the summary was produced. Only `Template` in this branch —
    /// the LLM variant lands in a future tier and is deliberately absent.
    pub method: CompactionMethod,
    /// Canonical ids covered by this summary, in canonical order.
    pub source_ids: Vec<MessageId>,
    /// The rendered summary text.
    pub text: String,
}

/// How a summary was produced. Intentionally single-variant in this
/// branch — an `Llm` variant would land alongside a future tier-2.
#[derive(Debug, Clone, PartialEq)]
pub enum CompactionMethod {
    /// Deterministic template render (no LLM call).
    Template,
}

/// Request-side directive slot reserved for a future microcompact
/// extension so the projector → request-builder signature won't change
/// when directives land.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestDirective {}

/// A request-time projection of a canonical `Context`. Always fully
/// constructed per-request, never persisted — no sidecar memoisation.
#[derive(Debug, Clone, PartialEq)]
pub struct Projection {
    /// Sequence-shaped output the request builder walks to assemble the
    /// provider DTO's message list.
    pub entries: Vec<ProjectedEntry>,
    /// Request-assembly directives applied after `entries` are walked.
    /// Always empty in this branch.
    pub directives: Vec<RequestDirective>,
}

/// Two-band tier selection. `Tier0` passes canonical through; `Tier1`
/// runs the forward-scan template projector with sliding summaries.
/// No `Tier2` variant — LLM summarisation is out of scope for this
/// branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Tier0,
    Tier1,
}

/// Projection-time configuration read from an `Agent` whose
/// `compaction_threshold` has already derived the token threshold.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionConfig {
    /// Tier-0 → tier-1 entry threshold (combined token count).
    pub effective_token_threshold: usize,
}

impl ProjectionConfig {
    /// Picks the tier for the request's combined `canonical + pending`
    /// token count. Callers compute the sum and pass it in.
    pub fn select_tier(&self, request_tokens: usize) -> Tier {
        if request_tokens >= self.effective_token_threshold {
            Tier::Tier1
        } else {
            Tier::Tier0
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

/// Entry point for building a projection from canonical context. Tier-1
/// (forward-scan template with sliding summaries) lands in a follow-up
/// file; this scaffolding provides the pass-through `Tier0` behaviour.
pub struct Projector;

impl Projector {
    /// Tier-0 pass-through. `Tier1` currently falls through to the
    /// pass-through body too — the forward-scan implementation is wired
    /// in alongside `Compactor` integration.
    pub fn project(
        context: &Context,
        _tier: Tier,
        _config: &ProjectionConfig,
    ) -> Projection {
        Projection {
            entries: context
                .messages
                .iter()
                .cloned()
                .map(|entry| ProjectedEntry::Original(Box::new(entry)))
                .collect(),
            directives: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::ContextMessage;
    use pretty_assertions::assert_eq;

    use super::*;

    fn config(tier_1: usize) -> ProjectionConfig {
        ProjectionConfig { effective_token_threshold: tier_1 }
    }

    /// Below threshold: `Tier0`. At or above: `Tier1`.
    #[test]
    fn test_select_tier_bands() {
        let cfg = config(100);
        assert_eq!(cfg.select_tier(0), Tier::Tier0);
        assert_eq!(cfg.select_tier(99), Tier::Tier0);
        assert_eq!(cfg.select_tier(100), Tier::Tier1);
        assert_eq!(cfg.select_tier(10_000), Tier::Tier1);
    }

    /// Scaffolding projector emits every canonical message as `Original`
    /// regardless of the requested tier — forward-scan behaviour lands
    /// in the algorithm module.
    #[test]
    fn test_projector_scaffolding_pass_through() {
        let fixture = Context::default().messages(vec![
            ContextMessage::user("hi", None).into(),
            ContextMessage::assistant("hello", None, None, None).into(),
        ]);
        let cfg = config(100);

        for tier in [Tier::Tier0, Tier::Tier1] {
            let actual = Projector::project(&fixture, tier, &cfg);
            assert_eq!(actual.entries.len(), 2);
            assert!(actual.directives.is_empty());
            for (expected, entry) in fixture.messages.iter().zip(&actual.entries) {
                match entry {
                    ProjectedEntry::Original(msg) => assert_eq!(msg.id, expected.id),
                    ProjectedEntry::Summary(_) => panic!("scaffolding emits only Original"),
                }
            }
        }
    }

    /// `ProjectionConfig::try_from(&Compact)` errors if the preprocessor
    /// has not written the derived threshold yet.
    #[test]
    fn test_projection_config_requires_derived_threshold() {
        let compact = Compact::new();
        let err = ProjectionConfig::try_from(&compact).unwrap_err();
        assert!(matches!(err, Error::ProjectionConfigNotReady));
    }

    /// Happy path: the derived threshold is populated and reads back verbatim.
    #[test]
    fn test_projection_config_reads_derived_value() {
        let mut compact = Compact::new();
        compact.token_threshold = Some(89_600);

        let cfg = ProjectionConfig::try_from(&compact).unwrap();

        assert_eq!(cfg.effective_token_threshold, 89_600);
    }

    /// Keep `SummaryPayload` constructible with a known `MessageId` so
    /// dead_code doesn't strip the type while the forward-scan
    /// algorithm is being wired in.
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
