use forge_tracker::{AiGenerationPayload, EventKind, ToolCallPayload};

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

/// For AI generation events with LLM telemetry data
pub fn ai_generation(payload: AiGenerationPayload) {
    dispatch(EventKind::AiGeneration(payload));
}
