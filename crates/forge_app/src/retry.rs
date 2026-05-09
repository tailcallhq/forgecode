use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};
use forge_config::RetryConfig;
use forge_domain::Error;

pub async fn retry_with_config<F, Fut, T, C>(
    config: &RetryConfig,
    operation: F,
    notify: Option<C>,
) -> anyhow::Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
    C: Fn(&anyhow::Error, Duration) + Send + Sync + 'static,
{
    let strategy = ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(config.min_delay_ms))
        .with_factor(config.backoff_factor as f32)
        .with_max_times(config.max_attempts)
        .with_jitter();

    let retryable = operation.retry(&strategy).when(should_retry);

    match notify {
        Some(callback) => retryable.notify(callback).await,
        None => retryable.await,
    }
}

/// Determines if an error should trigger a retry attempt.
///
/// This function checks if the error is a retryable domain error.
/// Currently, only `Error::Retryable` errors will trigger retries.
fn should_retry(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<Error>()
        .is_some_and(|error| matches!(error, Error::Retryable(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors how the WebSocket transport produces a retryable error: wrap an
    /// inner error chain in `Error::Retryable`, then `.into()` it to
    /// `anyhow::Error` so the orchestrator only sees `anyhow`. If
    /// `should_retry` ever returns false here, the orchestrator silently
    /// stops retrying every WebSocket failure.
    #[test]
    fn should_retry_recognizes_websocket_style_error() {
        let inner = anyhow::Error::msg("Connection reset without closing handshake")
            .context("OpenAI Responses WebSocket receive failed");
        let err: anyhow::Error = Error::Retryable(inner).into();
        assert!(should_retry(&err));
    }

    #[test]
    fn should_retry_recognizes_anyhow_wrapped_retryable() {
        // What if some code path adds an extra `.context()` on top of the
        // already-`Error::Retryable` anyhow? `downcast_ref` only finds the
        // top-level type; with an extra context layer, `should_retry`
        // returns false even though the chain is still retryable.
        let err: anyhow::Error = Error::Retryable(anyhow::Error::msg("inner")).into();
        let wrapped = err.context("Failed to process message stream");
        assert!(
            should_retry(&wrapped),
            "should_retry must walk the chain — adding any context layer must not silently disable retry"
        );
    }
}
