use crate::Event;

pub mod posthog;

/// Defines the interface for an event collector.
#[async_trait::async_trait]
pub trait Collect: Send + Sync {
    /// Collects a single event, sending it to the backend.
    async fn collect(&self, event: Event) -> super::Result<()>;
}
