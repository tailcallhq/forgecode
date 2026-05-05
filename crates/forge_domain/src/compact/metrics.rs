//! Compaction metrics tracking for monitoring and optimization.
//!
//! This module provides metrics collection for compaction operations,
//! enabling analysis of compaction patterns and optimization opportunities.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::ModelId;

/// Compaction event type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompactionEventType {
    /// Automatic compaction triggered by token threshold
    ThresholdExceeded,
    /// Automatic compaction triggered by message count
    MessageLimit,
    /// Manual compaction requested
    Manual,
    /// Pre-emptive compaction
    Preemptive,
}

/// Compaction summary strategy used
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SummaryStrategy {
    /// Extract-based summarization
    Extract,
    /// LLM-based summarization
    Llm,
    /// Hybrid summarization
    Hybrid,
}

/// Single compaction event record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionEvent {
    /// Timestamp when compaction started (milliseconds since Unix epoch)
    pub timestamp_ms: u64,
    /// Type of compaction event
    pub event_type: CompactionEventType,
    /// Summary strategy used
    pub summary_strategy: SummaryStrategy,
    /// Number of messages before compaction
    pub messages_before: usize,
    /// Number of messages after compaction
    pub messages_after: usize,
    /// Token count before compaction
    pub tokens_before: usize,
    /// Token count after compaction
    pub tokens_after: usize,
    /// Token reduction percentage
    pub reduction_percent: f64,
    /// Duration of compaction operation
    pub duration_ms: u64,
    /// Model used for LLM summarization (if applicable)
    pub model_used: Option<ModelId>,
    /// Whether compaction was successful
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
}

impl CompactionEvent {
    /// Create a new compaction event
    pub fn new(
        event_type: CompactionEventType,
        summary_strategy: SummaryStrategy,
        messages_before: usize,
        messages_after: usize,
        tokens_before: usize,
        tokens_after: usize,
        duration: Duration,
    ) -> Self {
        let reduction_percent = if tokens_before > 0 {
            ((tokens_before - tokens_after) as f64 / tokens_before as f64) * 100.0
        } else {
            0.0
        };

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            timestamp_ms,
            event_type,
            summary_strategy,
            messages_before,
            messages_after,
            tokens_before,
            tokens_after,
            reduction_percent,
            duration_ms: duration.as_millis() as u64,
            model_used: None,
            success: true,
            error: None,
        }
    }

    /// Mark event as failed
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.success = false;
        self.error = Some(error.into());
        self
    }

    /// Set the model used for summarization
    pub fn with_model(mut self, model: ModelId) -> Self {
        self.model_used = Some(model);
        self
    }
}

/// Compaction metrics collector
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactionMetrics {
    /// All compaction events
    events: Vec<CompactionEvent>,
    /// Compacted message count by strategy
    strategy_counts: HashMap<SummaryStrategy, usize>,
    /// Token reduction by strategy
    strategy_reduction: HashMap<SummaryStrategy, usize>,
    /// Event counts by type
    event_type_counts: HashMap<CompactionEventType, usize>,
    /// Total tokens saved
    total_tokens_saved: usize,
    /// Total messages saved
    total_messages_saved: usize,
    /// Compaction duration statistics (ms)
    total_duration_ms: u64,
    /// Failed compaction count
    failure_count: usize,
}

impl CompactionMetrics {
    /// Create new metrics collector
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a compaction event
    pub fn record(&mut self, event: CompactionEvent) {
        let strategy = event.summary_strategy;
        let event_type = event.event_type;
        let tokens_saved = event.tokens_before.saturating_sub(event.tokens_after);
        let messages_saved = event.messages_before.saturating_sub(event.messages_after);

        *self.strategy_counts.entry(strategy).or_insert(0) += 1;
        *self.strategy_reduction.entry(strategy).or_default() += tokens_saved;
        *self.event_type_counts.entry(event_type).or_insert(0) += 1;
        self.total_tokens_saved += tokens_saved;
        self.total_messages_saved += messages_saved;
        self.total_duration_ms += event.duration_ms;

        if !event.success {
            self.failure_count += 1;
        }

        self.events.push(event);
    }

    /// Get total compaction count
    pub fn total_compactions(&self) -> usize {
        self.events.len()
    }

    /// Get success rate
    pub fn success_rate(&self) -> f64 {
        if self.events.is_empty() {
            return 1.0;
        }
        let successes = self.events.len() - self.failure_count;
        successes as f64 / self.events.len() as f64
    }

    /// Get average token reduction percentage
    pub fn avg_reduction_percent(&self) -> f64 {
        if self.events.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.events.iter().map(|e| e.reduction_percent).sum();
        sum / self.events.len() as f64
    }

    /// Get average compaction duration in milliseconds
    pub fn avg_duration_ms(&self) -> f64 {
        if self.events.is_empty() {
            return 0.0;
        }
        self.total_duration_ms as f64 / self.events.len() as f64
    }

    /// Get total tokens saved
    pub fn total_tokens_saved(&self) -> usize {
        self.total_tokens_saved
    }

    /// Get total messages saved
    pub fn total_messages_saved(&self) -> usize {
        self.total_messages_saved
    }

    /// Get count by strategy
    pub fn count_by_strategy(&self, strategy: SummaryStrategy) -> usize {
        self.strategy_counts.get(&strategy).copied().unwrap_or(0)
    }

    /// Get count by event type
    pub fn count_by_event_type(&self, event_type: CompactionEventType) -> usize {
        self.event_type_counts.get(&event_type).copied().unwrap_or(0)
    }

    /// Get strategy with most usage
    pub fn most_used_strategy(&self) -> Option<SummaryStrategy> {
        self.strategy_counts
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(strategy, _)| *strategy)
    }

    /// Get the most recent events
    pub fn recent_events(&self, count: usize) -> Vec<&CompactionEvent> {
        self.events.iter().rev().take(count).collect()
    }

    /// Get events by strategy
    pub fn events_by_strategy(&self, strategy: SummaryStrategy) -> Vec<&CompactionEvent> {
        self.events.iter().filter(|e| e.summary_strategy == strategy).collect()
    }

    /// Clear all metrics
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_record_compaction_event() {
        let mut metrics = CompactionMetrics::new();

        let event = CompactionEvent::new(
            CompactionEventType::ThresholdExceeded,
            SummaryStrategy::Extract,
            100,
            20,
            50000,
            10000,
            Duration::from_millis(50),
        );

        metrics.record(event);

        assert_eq!(metrics.total_compactions(), 1);
        assert_eq!(metrics.total_tokens_saved(), 40000);
        assert_eq!(metrics.total_messages_saved(), 80);
        assert_eq!(metrics.avg_reduction_percent(), 80.0);
    }

    #[test]
    fn test_success_rate() {
        let mut metrics = CompactionMetrics::new();

        // Record successful event
        metrics.record(CompactionEvent::new(
            CompactionEventType::Manual,
            SummaryStrategy::Extract,
            10,
            5,
            5000,
            2500,
            Duration::ZERO,
        ));

        // Record failed event
        let failed = CompactionEvent::new(
            CompactionEventType::Manual,
            SummaryStrategy::Llm,
            10,
            5,
            5000,
            2500,
            Duration::ZERO,
        )
        .with_error("LLM timeout");
        metrics.record(failed);

        assert_eq!(metrics.success_rate(), 0.5);
    }

    #[test]
    fn test_most_used_strategy() {
        let mut metrics = CompactionMetrics::new();

        for _ in 0..3 {
            metrics.record(CompactionEvent::new(
                CompactionEventType::ThresholdExceeded,
                SummaryStrategy::Extract,
                10,
                5,
                5000,
                2500,
                Duration::ZERO,
            ));
        }

        for _ in 0..5 {
            metrics.record(CompactionEvent::new(
                CompactionEventType::Manual,
                SummaryStrategy::Hybrid,
                10,
                5,
                5000,
                2500,
                Duration::ZERO,
            ));
        }

        assert_eq!(metrics.most_used_strategy(), Some(SummaryStrategy::Hybrid));
    }
}
