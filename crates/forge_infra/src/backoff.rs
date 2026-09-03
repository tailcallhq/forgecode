//! Retry backoff strategies for transient upstream failures.
//!
//! [`Backoff`] maps a zero-based attempt index to a [`Duration`] delay.
//! Three strategies via [`BackoffStrategy`]:
//!
//! - [`Fixed`](BackoffStrategy::Fixed): constant delay regardless of attempt.
//! - [`Linear`](BackoffStrategy::Linear): `base * (attempt + 1)` — grows from
//!   attempt 0.
//! - [`Exponential`](BackoffStrategy::Exponential): `base * 2^attempt` —
//!   doubles.
//!
//! All strategies are capped at `max` so an unbounded exponential cannot
//! stall the caller. Saturating arithmetic prevents overflow at extreme
//! attempt counts.

use std::time::Duration;

/// Which growth pattern `Backoff` should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackoffStrategy {
    /// Constant delay (`base` regardless of attempt).
    Fixed,
    /// `base * (attempt + 1)` — grows linearly from attempt 0.
    Linear,
    /// `base * 2^attempt` — doubles per attempt.
    Exponential,
}

/// A configured retry schedule.
///
/// Capped at `max` so a runaway exponential does not stall the caller.
/// Construct with [`Backoff::new`] and query per attempt via
/// [`Backoff::delay_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    strategy: BackoffStrategy,
    base: Duration,
    max: Duration,
}

impl Backoff {
    /// Build a new schedule. `base` is the unit interval; `max` caps any
    /// individual delay.
    pub fn new(strategy: BackoffStrategy, base: Duration, max: Duration) -> Self {
        Self { strategy, base, max }
    }

    /// Delay for the given (zero-based) retry attempt.
    ///
    /// `attempt = 0` is the first retry — the delay BEFORE that retry runs.
    /// `attempt = 1` is the second retry (after one failure), etc.
    pub fn delay_for(&self, attempt: u32) -> Duration {
        let base_ms = self.base.as_millis() as u64;
        let max_ms = self.max.as_millis() as u64;
        let raw = match self.strategy {
            BackoffStrategy::Fixed => base_ms,
            BackoffStrategy::Linear => base_ms.saturating_mul(attempt as u64 + 1),
            BackoffStrategy::Exponential => {
                let shift = attempt.min(63);
                base_ms.saturating_mul(1u64 << shift)
            }
        };
        Duration::from_millis(raw.min(max_ms))
    }

    /// The active strategy.
    pub fn strategy(&self) -> BackoffStrategy {
        self.strategy
    }

    /// Base interval.
    pub fn base(&self) -> Duration {
        self.base
    }

    /// Maximum single delay.
    pub fn max(&self) -> Duration {
        self.max
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_constant() {
        let b = Backoff::new(
            BackoffStrategy::Fixed,
            Duration::from_millis(100),
            Duration::from_secs(10),
        );
        assert_eq!(b.delay_for(0), b.delay_for(5));
        assert_eq!(b.delay_for(0), Duration::from_millis(100));
    }

    #[test]
    fn linear_grows() {
        let b = Backoff::new(
            BackoffStrategy::Linear,
            Duration::from_millis(100),
            Duration::from_secs(10),
        );
        assert!(b.delay_for(2) > b.delay_for(0));
        assert_eq!(b.delay_for(0), Duration::from_millis(100));
        assert_eq!(b.delay_for(2), Duration::from_millis(300));
    }

    #[test]
    fn exponential_doubles() {
        let b = Backoff::new(
            BackoffStrategy::Exponential,
            Duration::from_millis(100),
            Duration::from_secs(10),
        );
        assert_eq!(b.delay_for(0).as_millis(), 100);
        assert_eq!(b.delay_for(1).as_millis(), 200);
        assert_eq!(b.delay_for(2).as_millis(), 400);
        assert_eq!(b.delay_for(3).as_millis(), 800);
    }

    #[test]
    fn capped_at_max() {
        let b = Backoff::new(
            BackoffStrategy::Exponential,
            Duration::from_millis(100),
            Duration::from_millis(500),
        );
        assert_eq!(b.delay_for(10).as_millis(), 500);
    }
}
