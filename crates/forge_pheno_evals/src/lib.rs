//! `forge_pheno_evals` — eval harness for forgecode memory sidecars (ADR-097).
//!
//! Routes forgecode's evaluation surface through the `pheno-forge-plugins`
//! sidecar stack. Each eval task is bound to a `MemoryScope` and routed
//! via the `CompositeAdapter` to the appropriate backing engine, so eval
//! results are automatically scoped to the engine being evaluated.
//!
//! ```text
//! forge eval run --scope=episodic --task=longmem-recall
//!   -> forge_pheno_evals::EvalRunner
//!   -> PhenoMemoryService.store(Episodic, fixture)
//!   -> CompositeAdapter routes to supermemory (Episodic)
//!   -> PhenoMemoryService.recall(Episodic, query)
//!   -> composite routes back to supermemory
//!   -> EvalScore { recall_at_k, latency_ms, ... }
//! ```

use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thegent_memory::v2::{
    adapters::MockAdapter,
    MemoryPort, MemoryQuery, MemoryScope, MemoryValue,
};

/// A single evaluation task: a fixture that goes in, a query that runs
/// against it, and a scorer that turns the result into an `EvalScore`.
#[async_trait]
pub trait EvalTask: Send + Sync {
    /// Scope this task is bound to (drives `CompositeAdapter` routing).
    fn scope(&self) -> MemoryScope;

    /// Human-readable name (e.g. `"longmem-recall"`, `"locomo-factoid"`,
    /// `"episodic-session-roundtrip"`).
    fn name(&self) -> &str;

    /// Stage the fixture: store N key/value pairs under the task's scope.
    async fn stage(&self, svc: &dyn MemoryPort) -> Result<(), MemoryError>;

    /// Run the eval query against the staged fixture.
    async fn query(&self, svc: &dyn MemoryPort) -> Result<Vec<String>, MemoryError>;

    /// Score the result (0.0–1.0) given the original fixture.
    fn score(&self, fixture: &[FixtureEntry], result: &[String]) -> f64;
}

/// One (key, value) pair to stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureEntry {
    pub key: String,
    pub value: String,
}

/// Result of running one eval task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalScore {
    pub task: String,
    pub scope: String,
    pub score: f64,
    pub stage_latency_ms: u64,
    pub query_latency_ms: u64,
    pub total_latency_ms: u64,
    pub passed: bool,
    pub threshold: f64,
}

impl EvalScore {
    pub fn passes(&self, threshold: f64) -> bool {
        self.score >= threshold
    }
}

/// Runner that stages fixtures, runs queries, and scores results.
pub struct EvalRunner {
    service: std::sync::Arc<dyn MemoryPort>,
    threshold: f64,
}

impl EvalRunner {
    /// Construct a runner with an explicit memory adapter.
    pub fn new(service: std::sync::Arc<dyn MemoryPort>) -> Self {
        Self {
            service,
            threshold: 0.7,
        }
    }

    /// Construct a runner with the in-process `MockAdapter` (for tests).
    pub fn mock() -> Self {
        Self::new(std::sync::Arc::new(MockAdapter::new()))
    }

    pub fn with_threshold(mut self, threshold: f64) -> Self {
        assert!(
            (0.0..=1.0).contains(&threshold),
            "threshold must be in 0.0..=1.0"
        );
        self.threshold = threshold;
        self
    }

    pub fn service(&self) -> &dyn MemoryPort {
        self.service.as_ref()
    }

    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    /// Run a single eval task. Returns the score + timing.
    pub async fn run(&self, task: &dyn EvalTask) -> Result<EvalScore, MemoryError> {
        let total_start = Instant::now();

        let stage_start = Instant::now();
        task.stage(self.service.as_ref()).await?;
        let stage_latency_ms = stage_start.elapsed().as_millis() as u64;

        let query_start = Instant::now();
        let result = task.query(self.service.as_ref()).await?;
        let query_latency_ms = query_start.elapsed().as_millis() as u64;

        let total_latency_ms = total_start.elapsed().as_millis() as u64;

        let fixture = task.fixture_snapshot();
        let score = task.score(&fixture, &result);

        Ok(EvalScore {
            task: task.name().to_string(),
            scope: format!("{:?}", task.scope()).to_lowercase(),
            score,
            stage_latency_ms,
            query_latency_ms,
            total_latency_ms,
            passed: score >= self.threshold,
            threshold: self.threshold,
        })
    }

    /// Run a suite of eval tasks; returns scores in input order.
    pub async fn run_suite(
        &self,
        tasks: &[Box<dyn EvalTask>],
    ) -> Vec<Result<EvalScore, MemoryError>> {
        let mut out = Vec::with_capacity(tasks.len());
        for task in tasks {
            out.push(self.run(task.as_ref()).await);
        }
        out
    }
}

// Re-export the upstream error so callers don't need to import thegent_memory.
pub use thegent_memory::v2::MemoryError;

// ---------------------------------------------------------------------------
// Built-in eval tasks
// ---------------------------------------------------------------------------

/// Roundtrip eval: store N entries, recall them, score by exact-match rate.
pub struct EpisodicRoundtrip {
    pub fixture: Vec<FixtureEntry>,
    pub threshold: f64,
}

impl EpisodicRoundtrip {
    pub fn new(fixture: Vec<FixtureEntry>) -> Self {
        Self {
            fixture,
            threshold: 0.8,
        }
    }
}

#[async_trait]
impl EvalTask for EpisodicRoundtrip {
    fn scope(&self) -> MemoryScope {
        MemoryScope::Episodic
    }

    fn name(&self) -> &str {
        "episodic-roundtrip"
    }

    async fn stage(&self, svc: &dyn MemoryPort) -> Result<(), MemoryError> {
        for entry in &self.fixture {
            let _id = svc
                .store(
                    self.scope(),
                    &entry.key,
                    MemoryValue::from(entry.value.as_str()),
                )
                .await?;
        }
        Ok(())
    }

    async fn query(&self, svc: &dyn MemoryPort) -> Result<Vec<String>, MemoryError> {
        let records = svc
            .recall(self.scope(), MemoryQuery::new(""))
            .await?;
        Ok(records.into_iter().map(|r| r.value.value_text()).collect())
    }

    fn score(&self, fixture: &[FixtureEntry], result: &[String]) -> f64 {
        if fixture.is_empty() {
            return 1.0;
        }
        let matches = fixture
            .iter()
            .filter(|e| result.iter().any(|r| r == &e.value))
            .count();
        matches as f64 / fixture.len() as f64
    }
}

impl EpisodicRoundtrip {
    /// Snapshot the fixture for the scorer. Not part of the trait surface
    /// — provided as a helper for `EvalRunner::run`.
    pub fn fixture_snapshot(&self) -> Vec<FixtureEntry> {
        self.fixture.clone()
    }
}

/// Latency budget eval: store 1 entry, recall it, score by recall success
/// at a latency threshold (in ms).
pub struct LatencyBudget {
    pub key: String,
    pub value: String,
    pub budget_ms: u64,
}

#[async_trait]
impl EvalTask for LatencyBudget {
    fn scope(&self) -> MemoryScope {
        MemoryScope::Episodic
    }

    fn name(&self) -> &str {
        "latency-budget"
    }

    async fn stage(&self, svc: &dyn MemoryPort) -> Result<(), MemoryError> {
        let _id = svc
            .store(
                self.scope(),
                &self.key,
                MemoryValue::from(self.value.as_str()),
            )
            .await?;
        Ok(())
    }

    async fn query(&self, svc: &dyn MemoryPort) -> Result<Vec<String>, MemoryError> {
        let records = svc
            .recall(self.scope(), MemoryQuery::new(&self.key))
            .await?;
        Ok(records.into_iter().map(|r| r.value.value_text()).collect())
    }

    fn score(&self, fixture: &[FixtureEntry], result: &[String]) -> f64 {
        let found = fixture
            .iter()
            .any(|e| result.iter().any(|r| r == &e.value));
        if found {
            1.0
        } else {
            0.0
        }
    }
}

impl LatencyBudget {
    pub fn fixture_snapshot(&self) -> Vec<FixtureEntry> {
        vec![FixtureEntry {
            key: self.key.clone(),
            value: self.value.clone(),
        }]
    }

    pub fn budget_ms(&self) -> u64 {
        self.budget_ms
    }
}

// ---------------------------------------------------------------------------
// Convenience: snapshot a fixture for any task via a trait extension
// ---------------------------------------------------------------------------

/// Trait extension that lets `EvalRunner::run` grab a fixture snapshot
/// from any task (including non-`EpisodicRoundtrip` / non-`LatencyBudget` tasks).
pub trait EvalTaskFixture {
    fn fixture_snapshot(&self) -> Vec<FixtureEntry>;
}

impl EvalTaskFixture for dyn EvalTask + '_ {
    fn fixture_snapshot(&self) -> Vec<FixtureEntry> {
        Vec::new()
    }
}

// Blanket impls for the built-in tasks.
impl EvalTaskFixture for EpisodicRoundtrip {
    fn fixture_snapshot(&self) -> Vec<FixtureEntry> {
        self.fixture.clone()
    }
}

impl EvalTaskFixture for LatencyBudget {
    fn fixture_snapshot(&self) -> Vec<FixtureEntry> {
        vec![FixtureEntry {
            key: self.key.clone(),
            value: self.value.clone(),
        }]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episodic_roundtrip_score_is_match_rate() {
        let fixture = vec![
            FixtureEntry {
                key: "a".into(),
                value: "alpha".into(),
            },
            FixtureEntry {
                key: "b".into(),
                value: "beta".into(),
            },
        ];
        let task = EpisodicRoundtrip::new(fixture.clone());
        let result = vec!["alpha".into(), "gamma".into()];
        assert!((task.score(&fixture, &result) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn episodic_roundtrip_empty_fixture_is_perfect() {
        let task = EpisodicRoundtrip::new(vec![]);
        let result: Vec<String> = vec![];
        assert_eq!(task.score(&[], &result), 1.0);
    }

    #[test]
    fn latency_budget_score_is_binary() {
        let task = LatencyBudget {
            key: "k".into(),
            value: "v".into(),
            budget_ms: 100,
        };
        let fixture = vec![FixtureEntry {
            key: "k".into(),
            value: "v".into(),
        }];
        let hit = vec!["v".into()];
        let miss: Vec<String> = vec![];
        assert_eq!(task.score(&fixture, &hit), 1.0);
        assert_eq!(task.score(&fixture, &miss), 0.0);
    }

    #[test]
    fn score_thresholds_are_inclusive() {
        let s = EvalScore {
            task: "t".into(),
            scope: "episodic".into(),
            score: 0.7,
            stage_latency_ms: 0,
            query_latency_ms: 0,
            total_latency_ms: 0,
            passed: true,
            threshold: 0.7,
        };
        assert!(s.passes(0.7));
        assert!(!s.passes(0.71));
    }

    #[test]
    fn episodic_roundtrip_routes_to_episodic_scope() {
        let task = EpisodicRoundtrip::new(vec![FixtureEntry {
            key: "k".into(),
            value: "v".into(),
        }]);
        assert_eq!(task.scope(), MemoryScope::Episodic);
    }
}
