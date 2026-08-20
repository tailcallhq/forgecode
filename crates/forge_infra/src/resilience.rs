//! Resilience primitives: circuit breaker and concurrency bulkhead.
//!
//! # Circuit breaker
//!
//! Wraps call sites that can fail transiently (MCP servers, provider HTTP
//! endpoints). After `failure_threshold` consecutive failures the breaker
//! **opens** and every call fails immediately with [`CircuitOpenError`].
//! After `reset_timeout` the breaker enters a **half-open** probe: the next
//! call is allowed through.  On success the breaker **closes** again; on
//! failure it re-opens and the timer resets.
//!
//! All state is held behind an `Arc<Mutex<…>>` so the breaker can be cloned
//! cheaply and shared across tasks.
//!
//! # Bulkhead
//!
//! A thin wrapper around [`tokio::sync::Semaphore`] that bounds how many
//! concurrent calls reach the protected resource.  When the semaphore is
//! exhausted the caller receives [`BulkheadFullError`] immediately (no
//! queue-and-wait semantics — this is intentional: callers must be aware that
//! the downstream is saturated).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::{Semaphore, SemaphorePermit};
use tracing::{debug, warn};

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
#[error("circuit breaker is open for {name:?}; retry after reset timeout")]
pub struct CircuitOpenError {
    pub name: String,
}

#[derive(Debug, Error)]
#[error("bulkhead for {name:?} is at capacity ({max_concurrent} concurrent calls)")]
pub struct BulkheadFullError {
    pub name: String,
    pub max_concurrent: usize,
}

// ── Circuit breaker
// ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakerState {
    Closed,
    Open { since: Instant },
    HalfOpen,
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before the breaker opens.
    pub failure_threshold: u32,
    /// How long the breaker stays open before allowing a probe.
    pub reset_timeout: Duration,
    /// Human-readable name used in logs and errors.
    pub name: String,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(30),
            name: "unnamed".to_string(),
        }
    }
}

struct BreakerInner {
    state: BreakerState,
    consecutive_failures: u32,
    config: CircuitBreakerConfig,
}

impl BreakerInner {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self { state: BreakerState::Closed, consecutive_failures: 0, config }
    }

    /// Returns `true` if the call should be allowed through.
    fn allow_call(&mut self) -> bool {
        match self.state {
            BreakerState::Closed => true,
            BreakerState::HalfOpen => false, // already probing
            BreakerState::Open { since } => {
                if since.elapsed() >= self.config.reset_timeout {
                    debug!(name = %self.config.name, "circuit breaker entering half-open");
                    self.state = BreakerState::HalfOpen;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn on_success(&mut self) {
        if matches!(
            self.state,
            BreakerState::HalfOpen | BreakerState::Open { .. }
        ) {
            debug!(name = %self.config.name, "circuit breaker closing after probe success");
        }
        self.state = BreakerState::Closed;
        self.consecutive_failures = 0;
    }

    fn on_failure(&mut self) {
        self.consecutive_failures += 1;
        let threshold = self.config.failure_threshold;
        match self.state {
            BreakerState::Closed if self.consecutive_failures >= threshold => {
                warn!(
                    name = %self.config.name,
                    failures = self.consecutive_failures,
                    "circuit breaker opening after {threshold} consecutive failures"
                );
                self.state = BreakerState::Open { since: Instant::now() };
            }
            BreakerState::HalfOpen => {
                warn!(name = %self.config.name, "circuit breaker re-opening after probe failure");
                self.state = BreakerState::Open { since: Instant::now() };
            }
            _ => {}
        }
    }
}

/// A cloneable, async-safe circuit breaker.
#[derive(Clone)]
pub struct CircuitBreaker {
    inner: Arc<Mutex<BreakerInner>>,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self { inner: Arc::new(Mutex::new(BreakerInner::new(config))) }
    }

    /// Execute `f`, tracking success/failure for the breaker.
    ///
    /// Returns `Err(CircuitOpenError)` immediately when the breaker is open and
    /// the reset timeout has not yet elapsed.
    pub async fn call<F, Fut, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        let name = {
            let mut inner = self.inner.lock().unwrap();
            if !inner.allow_call() {
                let name = inner.config.name.clone();
                return Err(CircuitOpenError { name }.into());
            }
            inner.config.name.clone()
        };

        let result = f().await;

        {
            let mut inner = self.inner.lock().unwrap();
            match &result {
                Ok(_) => inner.on_success(),
                Err(_) => inner.on_failure(),
            }
        }

        debug!(name = %name, ok = result.is_ok(), "circuit breaker call completed");
        result
    }

    /// Current state as a string — for observability / tests.
    pub fn state_name(&self) -> &'static str {
        match self.inner.lock().unwrap().state {
            BreakerState::Closed => "closed",
            BreakerState::Open { .. } => "open",
            BreakerState::HalfOpen => "half-open",
        }
    }

    /// Number of consecutive failures tracked so far.
    pub fn consecutive_failures(&self) -> u32 {
        self.inner.lock().unwrap().consecutive_failures
    }
}

// ── Bulkhead
// ──────────────────────────────────────────────────────────────────

/// A concurrency bulkhead backed by a bounded semaphore.
///
/// Callers that cannot acquire a permit immediately receive
/// [`BulkheadFullError`] — there is no queue.
#[derive(Clone)]
pub struct Bulkhead {
    semaphore: Arc<Semaphore>,
    max_concurrent: usize,
    name: String,
}

impl Bulkhead {
    pub fn new(name: impl Into<String>, max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
            name: name.into(),
        }
    }

    /// Try to acquire a permit. Returns immediately with an error if saturated.
    pub fn try_acquire(&self) -> anyhow::Result<SemaphorePermit<'_>> {
        self.semaphore.try_acquire().map_err(|_| {
            BulkheadFullError { name: self.name.clone(), max_concurrent: self.max_concurrent }
                .into()
        })
    }

    /// Execute `f` under the bulkhead, failing immediately if at capacity.
    pub async fn call<F, Fut, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        let _permit = self.try_acquire()?;
        f().await
    }

    /// How many permits are currently available (for observability / tests).
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    fn breaker(threshold: u32, reset_ms: u64) -> CircuitBreaker {
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: threshold,
            reset_timeout: Duration::from_millis(reset_ms),
            name: "test".to_string(),
        })
    }

    // ── Circuit breaker ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn breaker_opens_after_threshold_failures() {
        let cb = breaker(3, 10_000);

        // 3 consecutive failures → breaker opens
        for _ in 0..3 {
            let _ = cb
                .call(|| async { Err::<(), _>(anyhow::anyhow!("fail")) })
                .await;
        }

        assert_eq!(cb.state_name(), "open");
        assert_eq!(cb.consecutive_failures(), 3);

        // Next call must be rejected immediately
        let err = cb
            .call(|| async { Ok::<(), anyhow::Error>(()) })
            .await
            .unwrap_err();
        assert!(err.downcast_ref::<CircuitOpenError>().is_some());
    }

    #[tokio::test]
    async fn breaker_closes_after_successful_probe() {
        let cb = breaker(2, 1); // 1 ms reset timeout

        for _ in 0..2 {
            let _ = cb
                .call(|| async { Err::<(), _>(anyhow::anyhow!("fail")) })
                .await;
        }
        assert_eq!(cb.state_name(), "open");

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Probe succeeds → closed
        cb.call(|| async { Ok::<(), anyhow::Error>(()) })
            .await
            .unwrap();
        assert_eq!(cb.state_name(), "closed");
        assert_eq!(cb.consecutive_failures(), 0);
    }

    #[tokio::test]
    async fn breaker_reopens_on_probe_failure() {
        let cb = breaker(2, 1);

        for _ in 0..2 {
            let _ = cb
                .call(|| async { Err::<(), _>(anyhow::anyhow!("fail")) })
                .await;
        }

        tokio::time::sleep(Duration::from_millis(5)).await;

        // Probe fails → re-opens
        let _ = cb
            .call(|| async { Err::<(), _>(anyhow::anyhow!("still failing")) })
            .await;
        assert_eq!(cb.state_name(), "open");
    }

    #[tokio::test]
    async fn breaker_resets_failure_count_on_success() {
        let cb = breaker(5, 10_000);

        // Two failures then a success → counter resets
        for _ in 0..2 {
            let _ = cb
                .call(|| async { Err::<(), _>(anyhow::anyhow!("fail")) })
                .await;
        }
        cb.call(|| async { Ok::<(), anyhow::Error>(()) })
            .await
            .unwrap();

        assert_eq!(cb.consecutive_failures(), 0);
        assert_eq!(cb.state_name(), "closed");
    }

    // ── Bulkhead ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn bulkhead_allows_up_to_limit() {
        let bh = Bulkhead::new("test", 2);

        let p1 = bh.try_acquire().unwrap();
        let p2 = bh.try_acquire().unwrap();

        // At capacity
        assert!(bh.try_acquire().is_err());

        drop(p1);
        // One permit freed
        let _p3 = bh.try_acquire().unwrap();
        drop(p2);
    }

    #[tokio::test]
    async fn bulkhead_call_rejects_when_full() {
        let bh = Bulkhead::new("test", 1);

        let _permit = bh.try_acquire().unwrap();

        let err = bh
            .call(|| async { Ok::<(), anyhow::Error>(()) })
            .await
            .unwrap_err();
        assert!(err.downcast_ref::<BulkheadFullError>().is_some());
    }

    #[tokio::test]
    async fn bulkhead_releases_permit_after_call() {
        let bh = Bulkhead::new("test", 1);

        bh.call(|| async { Ok::<(), anyhow::Error>(()) })
            .await
            .unwrap();

        // Permit should be released
        assert_eq!(bh.available_permits(), 1);
    }

    #[tokio::test]
    async fn bulkhead_concurrent_calls_bounded() {
        use std::sync::Arc;

        use tokio::sync::Barrier;

        let bh = Bulkhead::new("test", 3);
        let counter = Arc::new(AtomicU32::new(0));
        let barrier = Arc::new(Barrier::new(3));

        let mut handles = vec![];
        for _ in 0..3 {
            let bh = bh.clone();
            let counter = counter.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                bh.call(|| {
                    let counter = counter.clone();
                    let barrier = barrier.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        barrier.wait().await;
                        Ok::<(), anyhow::Error>(())
                    }
                })
                .await
            }));
        }

        for h in handles {
            h.await.unwrap().unwrap();
        }

        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
