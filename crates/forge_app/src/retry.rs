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
