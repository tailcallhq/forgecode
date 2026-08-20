use std::sync::Arc;

use forge_similarity::SimilarityProvider;
use tokio::sync::broadcast;

use crate::config::{ApprovalMode, DriftConfig, Tier};
use crate::event::{AlertId, DriftEvent, OverrideReason};
use crate::index::DriftIndex;

/// High-level drift detector that wraps a `DriftIndex` and an optional
/// `SimilarityProvider`.  T0–T3 tiers degrade gracefully.
pub struct DriftDetector {
    config: DriftConfig,
    index: Arc<DriftIndex>,
    similarity: Option<Arc<dyn SimilarityProvider>>,
    tx: broadcast::Sender<DriftEvent>,
}

impl DriftDetector {
    /// Build a new detector.
    ///
    /// * `tier` — T0 (hash only), T1 (+Jaccard), T2/T3 (+embedding).
    /// * `similarity` — optional embedding-based similarity (T2/T3).
    /// * `index` — shared index (can be shared with forge3d `Registry`).
    pub fn new(
        config: DriftConfig,
        index: Arc<DriftIndex>,
        similarity: Option<Arc<dyn SimilarityProvider>>,
    ) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { config, index, similarity, tx }
    }

    /// Subscribe to drift events.
    pub fn subscribe(&self) -> broadcast::Receiver<DriftEvent> {
        self.tx.subscribe()
    }

    // ------------------------------------------------------------------
    // Public entry point
    // ------------------------------------------------------------------

    /// Observe a new prompt for `agent_id` and return a `DriftEvent` if
    /// an overlap is detected.  Tier masks automatically: if T2/T3
    /// similarity is not available, it falls back to T1 then T0.
    pub async fn observe(
        &self,
        agent_id: &str,
        prompt: &str,
        lane: &str,
        _now_ms: i64,
    ) -> Option<DriftEvent> {
        if matches!(self.config.approval_mode, ApprovalMode::Off) {
            return None;
        }

        let threshold = self.config.threshold;
        self.index.observe(agent_id, prompt);

        // ---------- T0 : exact hash match ----------
        if matches!(self.config.tier, Tier::T0) {
            return self.tier0_or_higher(agent_id, prompt, lane, threshold);
        }

        // ---------- T1+ : similarity scan via optional provider ----------
        let sim = match self.similarity.as_ref() {
            Some(provider) => Some(provider.compare(agent_id, prompt).await),
            None => None,
        };

        match sim {
            Some(Ok(Some(score))) => {
                // T2/T3 tier path via extern provider
                self.emit_if_above(agent_id, prompt, lane, threshold, score)
            }
            Some(Ok(None)) | Some(Err(_)) => {
                // Provider declined, unconfigured, or errored → Jaccard fallback
                self.jaccard_fallback(agent_id, prompt, lane, threshold)
            }
            None => {
                // No similarity provider at all → Jaccard fallback
                self.jaccard_fallback(agent_id, prompt, lane, threshold)
            }
        }
    }

    /// Override / ack an alert.
    pub fn override_alert(&self, id: AlertId, reason: OverrideReason) {
        let _ = self.tx.send(DriftEvent::OverrideApplied { id, reason });
    }

    // ------------------------------------------------------------------
    // internals
    // ------------------------------------------------------------------

    fn tier0_or_higher(
        &self,
        agent_id: &str,
        prompt: &str,
        lane: &str,
        _threshold: f64,
    ) -> Option<DriftEvent> {
        if self.index.is_exact_match(agent_id, prompt) {
            let id = AlertId::next();
            let ev = DriftEvent::OverlapAlert {
                id,
                agent_id: agent_id.to_string(),
                similarity: 1.0,
                lane: lane.to_string(),
                prompt_excerpt: prompt.chars().take(80).collect(),
            };
            let _ = self.tx.send(ev.clone());
            Some(ev)
        } else {
            None
        }
    }

    fn jaccard_fallback(
        &self,
        agent_id: &str,
        prompt: &str,
        lane: &str,
        threshold: f64,
    ) -> Option<DriftEvent> {
        let score = self.index.jaccard(agent_id, prompt).unwrap_or(0.0);
        self.emit_if_above(agent_id, prompt, lane, threshold, score)
    }

    fn emit_if_above(
        &self,
        agent_id: &str,
        prompt: &str,
        lane: &str,
        threshold: f64,
        similarity: f64,
    ) -> Option<DriftEvent> {
        if similarity < threshold {
            return None;
        }
        let id = AlertId::next();
        let ev = DriftEvent::OverlapAlert {
            id,
            agent_id: agent_id.to_string(),
            similarity,
            lane: lane.to_string(),
            prompt_excerpt: prompt.chars().take(80).collect(),
        };
        let _ = self.tx.send(ev.clone());
        Some(ev)
    }
}
