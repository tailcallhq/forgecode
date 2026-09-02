use forge_tracker::{EventKind, ToolCallPayload};

use crate::TRACKER;

/// Helper functions to eliminate duplication of tokio::spawn + TRACKER patterns
/// Generic dispatcher for any event
fn dispatch(event: EventKind) {
    tokio::spawn(TRACKER.dispatch(event));
}

/// Dispatches an event blockingly
/// This is useful for events that are not expected to be dispatched in the
/// background
fn dispatch_blocking(event: EventKind) {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(TRACKER.dispatch(event))
    })
    .ok();
}

/// For error events with Debug formatting
pub fn error<E: std::fmt::Debug>(error: E) {
    dispatch(EventKind::Error(format!("{error:?}")));
}

pub fn error_blocking<E: std::fmt::Debug>(error: E) {
    dispatch_blocking(EventKind::Error(format!("{error:?}")));
}

/// For error events with string input
pub fn error_string(error: String) {
    dispatch(EventKind::Error(error));
}

/// For tool call events
pub fn tool_call(payload: ToolCallPayload) {
    dispatch(EventKind::ToolCall(payload));
}

/// For prompt events
pub fn prompt(text: String) {
    dispatch(EventKind::Prompt(text));
}

/// For model setting
pub fn set_model(model: String) {
    tokio::spawn(TRACKER.set_model(model));
}

pub fn login(login: String) {
    tokio::spawn(TRACKER.login(login));
}

/// Initialises file logging for entry points that do not go through the
/// interactive UI (`forge machine stdio`). Keep the returned guard alive for
/// the life of the process.
pub fn init_tracing(log_path: std::path::PathBuf) -> anyhow::Result<forge_tracker::Guard> {
    forge_tracker::init_tracing(log_path, TRACKER.clone())
}
