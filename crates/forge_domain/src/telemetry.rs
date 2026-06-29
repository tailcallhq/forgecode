/// Pluggable telemetry facade for ForgeCode.
///
/// The default implementation is a no-op so that callers never need to
/// configure a backend. When a real sink is wired (e.g. in `forge_main` via
/// `tracing-subscriber`), spans and counters become visible automatically.
///
/// # Design
/// - `MetricsSink` is a lightweight trait with a blanket no-op.
/// - `NoopMetricsSink` is the zero-cost default.
/// - Counters are named after hot paths: `request`, `model_exec`, `stream`,
///   `retry`, `tool_call`.
/// - No Prometheus or external dep is required.
use std::time::Duration;

/// A counter/timer sink for hot-path telemetry.
///
/// All methods have default no-op bodies so implementors only override what
/// they need.
pub trait MetricsSink: Send + Sync + 'static {
    /// Increment a named counter by `delta`.
    fn increment(&self, name: &'static str, delta: u64) {
        let _ = (name, delta);
    }

    /// Record a duration for a named operation.
    fn record_duration(&self, name: &'static str, duration: Duration) {
        let _ = (name, duration);
    }

    /// Record an error for a named operation.
    fn record_error(&self, name: &'static str) {
        let _ = name;
    }
}

/// The zero-cost default sink — all methods are no-ops compiled away.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopMetricsSink;

impl MetricsSink for NoopMetricsSink {}

/// Well-known metric names used on the hot paths.
pub mod metric_names {
    /// A chat/completion request was dispatched to a provider.
    pub const REQUEST: &str = "forge.request";
    /// A model execution (LLM inference) completed.
    pub const MODEL_EXEC: &str = "forge.model_exec";
    /// A streaming response chunk was received.
    pub const STREAM_CHUNK: &str = "forge.stream.chunk";
    /// A request was retried.
    pub const RETRY: &str = "forge.retry";
    /// A tool call was dispatched.
    pub const TOOL_CALL: &str = "forge.tool_call";
    /// A tool call produced an error.
    pub const TOOL_ERROR: &str = "forge.tool_call.error";
}
