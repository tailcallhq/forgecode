//! Helpers to handle connection delays when receiving errors

use std::time::Duration;

use crate::error::Error;
#[cfg(doc)]
use crate::event_source::{Event, EventSource};

/// Describes how an [`EventSource`] should retry on receiving an [`enum@Error`]
pub trait RetryPolicy {
    /// Submit a new retry delay based on the [`enum@Error`], last retry number
    /// and duration, if available. A policy may also return `None` if it
    /// does not want to retry
    fn retry(&self, error: &Error, last_retry: Option<(usize, Duration)>) -> Option<Duration>;

    /// Set a new reconnection time if received from an [`Event`]
    fn set_reconnection_time(&mut self, duration: Duration);
}

/// A [`RetryPolicy`] which backs off exponentially
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    /// The start of the backoff
    pub start: Duration,
    /// The factor of which to backoff by
    pub factor: f64,
    /// The maximum duration to delay
    pub max_duration: Option<Duration>,
    /// The maximum number of retries before giving up
    pub max_retries: Option<usize>,
}

impl ExponentialBackoff {
    /// Create a new exponential backoff retry policy
    pub const fn new(
        start: Duration,
        factor: f64,
        max_duration: Option<Duration>,
        max_retries: Option<usize>,
    ) -> Self {
        Self { start, factor, max_duration, max_retries }
    }
}

impl RetryPolicy for ExponentialBackoff {
    fn retry(&self, _error: &Error, last_retry: Option<(usize, Duration)>) -> Option<Duration> {
        if let Some((retry_num, last_duration)) = last_retry {
            if self.max_retries.is_none() || retry_num < self.max_retries.unwrap() {
                let duration = last_duration.mul_f64(self.factor);
                if let Some(max_duration) = self.max_duration {
                    Some(duration.min(max_duration))
                } else {
                    Some(duration)
                }
            } else {
                None
            }
        } else {
            Some(self.start)
        }
    }
    fn set_reconnection_time(&mut self, duration: Duration) {
        self.start = duration;
        if let Some(max_duration) = self.max_duration {
            self.max_duration = Some(max_duration.max(duration))
        }
    }
}

/// A [`RetryPolicy`] which always emits the same delay
#[derive(Debug, Clone)]
pub struct Constant {
    /// The delay to return
    pub delay: Duration,
    /// The maximum number of retries to return before giving up
    pub max_retries: Option<usize>,
}

impl Constant {
    /// Create a new constant retry policy
    pub const fn new(delay: Duration, max_retries: Option<usize>) -> Self {
        Self { delay, max_retries }
    }
}

impl RetryPolicy for Constant {
    fn retry(&self, _error: &Error, last_retry: Option<(usize, Duration)>) -> Option<Duration> {
        if let Some((retry_num, _)) = last_retry {
            if self.max_retries.is_none() || retry_num < self.max_retries.unwrap() {
                Some(self.delay)
            } else {
                None
            }
        } else {
            Some(self.delay)
        }
    }
    fn set_reconnection_time(&mut self, duration: Duration) {
        self.delay = duration;
    }
}

/// A [`RetryPolicy`] which never retries
#[derive(Debug, Clone, Copy, Default)]
pub struct Never;

impl RetryPolicy for Never {
    fn retry(&self, _error: &Error, _last_retry: Option<(usize, Duration)>) -> Option<Duration> {
        None
    }
    fn set_reconnection_time(&mut self, _duration: Duration) {}
}

/// The default [`RetryPolicy`] when initializing an [`EventSource`]
pub const DEFAULT_RETRY: ExponentialBackoff = ExponentialBackoff::new(
    Duration::from_millis(300),
    2.,
    Some(Duration::from_secs(5)),
    None,
);

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// Reusable error fixture; all policies ignore the error contents.
    fn error_fixture() -> Error {
        Error::StreamEnded
    }

    /// Reusable exponential backoff fixture starting at 100ms, doubling.
    fn backoff_fixture() -> ExponentialBackoff {
        ExponentialBackoff::new(Duration::from_millis(100), 2.0, None, None)
    }

    #[test]
    fn test_exponential_first_retry_uses_start() {
        let fixture = backoff_fixture();

        let actual = fixture.retry(&error_fixture(), None);

        let expected = Some(Duration::from_millis(100));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_exponential_multiplies_last_duration_by_factor() {
        let fixture = backoff_fixture();

        let actual = fixture.retry(&error_fixture(), Some((1, Duration::from_millis(100))));

        let expected = Some(Duration::from_millis(200));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_exponential_clamps_to_max_duration() {
        let fixture = ExponentialBackoff::new(
            Duration::from_millis(100),
            2.0,
            Some(Duration::from_millis(150)),
            None,
        );

        let actual = fixture.retry(&error_fixture(), Some((1, Duration::from_millis(100))));

        let expected = Some(Duration::from_millis(150));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_exponential_stops_at_max_retries() {
        let fixture = ExponentialBackoff::new(Duration::from_millis(100), 2.0, None, Some(3));

        let actual = fixture.retry(&error_fixture(), Some((3, Duration::from_millis(100))));

        let expected = None;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_exponential_retries_below_max_retries() {
        let fixture = ExponentialBackoff::new(Duration::from_millis(100), 2.0, None, Some(3));

        let actual = fixture.retry(&error_fixture(), Some((2, Duration::from_millis(100))));

        let expected = Some(Duration::from_millis(200));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_exponential_set_reconnection_time_raises_start_and_max() {
        let mut fixture = ExponentialBackoff::new(
            Duration::from_millis(100),
            2.0,
            Some(Duration::from_millis(150)),
            Some(5),
        );

        fixture.set_reconnection_time(Duration::from_millis(400));
        let actual = (fixture.start, fixture.max_duration, fixture.max_retries);

        let expected = (
            Duration::from_millis(400),
            Some(Duration::from_millis(400)),
            Some(5),
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_exponential_set_reconnection_time_keeps_larger_max() {
        let mut fixture = ExponentialBackoff::new(
            Duration::from_millis(100),
            2.0,
            Some(Duration::from_secs(5)),
            None,
        );

        fixture.set_reconnection_time(Duration::from_millis(400));
        let actual = (fixture.start, fixture.max_duration);

        let expected = (Duration::from_millis(400), Some(Duration::from_secs(5)));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_constant_returns_same_delay_regardless_of_last_retry() {
        let fixture = Constant::new(Duration::from_millis(250), None);

        let actual = (
            fixture.retry(&error_fixture(), None),
            fixture.retry(&error_fixture(), Some((9, Duration::from_secs(60)))),
        );

        let expected = (
            Some(Duration::from_millis(250)),
            Some(Duration::from_millis(250)),
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_constant_stops_at_max_retries() {
        let fixture = Constant::new(Duration::from_millis(250), Some(2));

        let actual = fixture.retry(&error_fixture(), Some((2, Duration::from_millis(250))));

        let expected = None;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_constant_set_reconnection_time_replaces_delay() {
        let mut fixture = Constant::new(Duration::from_millis(250), None);

        fixture.set_reconnection_time(Duration::from_millis(900));
        let actual = fixture.retry(&error_fixture(), None);

        let expected = Some(Duration::from_millis(900));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_never_policy_never_retries() {
        let mut fixture = Never;
        fixture.set_reconnection_time(Duration::from_secs(1));

        let actual = (
            fixture.retry(&error_fixture(), None),
            fixture.retry(&error_fixture(), Some((0, Duration::from_secs(1)))),
        );

        let expected = (None, None);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_default_retry_constant_values() {
        let fixture = DEFAULT_RETRY;

        let actual = (
            fixture.start,
            fixture.factor,
            fixture.max_duration,
            fixture.max_retries,
        );

        let expected = (
            Duration::from_millis(300),
            2.0,
            Some(Duration::from_secs(5)),
            None,
        );
        assert_eq!(actual, expected);
    }
}
