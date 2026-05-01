use posthog_rs::{Client, ClientOptions, Event as PhEvent};
use tokio::sync::OnceCell;

use super::super::Result;
use super::Collect;
use crate::Event;

/// PostHog event collector backed by the official `posthog-rs` SDK.
///
/// The SDK client is constructed lazily on first use so that `new` can remain
/// synchronous and work correctly inside `Default` / `LazyLock` initialisers.
pub struct Tracker {
    api_secret: String,
    client: OnceCell<Client>,
}

impl Tracker {
    /// Creates a new PostHog tracker that will use the provided API secret.
    pub fn new(api_secret: &str) -> Self {
        Self { api_secret: api_secret.to_string(), client: OnceCell::new() }
    }

    async fn get_client(&self) -> &Client {
        self.client
            .get_or_init(|| async {
                let options = ClientOptions::from(self.api_secret.as_str());
                posthog_rs::client(options).await
            })
            .await
    }
}

/// Converts an internal [`Event`] into a [`posthog_rs::Event`].
fn build_ph_event(input: &Event) -> Result<PhEvent> {
    let distinct_id = input.client_id.clone();
    let event_name = input.event_name.to_string();
    let mut ph_event = PhEvent::new(event_name, distinct_id);

    ph_event.insert_prop("event_value", &input.event_value)?;
    ph_event.insert_prop("cores", input.cores)?;
    ph_event.insert_prop("os_name", &input.os_name)?;
    ph_event.insert_prop("up_time", input.up_time)?;
    ph_event.insert_prop("version", &input.version)?;
    ph_event.insert_prop("user", &input.user)?;
    ph_event.insert_prop("start_time", input.start_time.to_rfc3339())?;
    let _ = ph_event.set_timestamp(input.start_time);

    if !input.email.is_empty() {
        ph_event.insert_prop("email", &input.email)?;
    }
    if !input.args.is_empty() {
        ph_event.insert_prop("args", &input.args)?;
    }
    if let Some(path) = &input.path {
        ph_event.insert_prop("path", path)?;
    }
    if let Some(cwd) = &input.cwd {
        ph_event.insert_prop("cwd", cwd)?;
    }
    if let Some(model) = &input.model {
        ph_event.insert_prop("model", model)?;
    }
    if let Some(conversation) = &input.conversation
        && let Ok(value) = serde_json::to_value(conversation)
    {
        ph_event.insert_prop("conversation", value)?;
    }
    // $set sends person properties to PostHog for user identification.
    if let Some(identity) = &input.identity
        && let Ok(value) = serde_json::to_value(identity)
    {
        ph_event.insert_prop("$set", value)?;
    }

    // Map AiGeneration payload to native PostHog $ai_generation schema.
    if &*input.event_name == "ai_generation"
        && let Ok(payload) =
            serde_json::from_str::<crate::AiGenerationPayload>(&input.event_value)
    {
        ph_event.insert_prop("$ai_provider", &payload.provider)?;
        ph_event.insert_prop("$ai_model", &payload.model)?;
        ph_event.insert_prop("$ai_input_tokens", payload.input_tokens)?;
        ph_event.insert_prop("$ai_output_tokens", payload.output_tokens)?;
        ph_event.insert_prop("$ai_latency", payload.latency_ms / 1000.0)?;
        ph_event.insert_prop("$ai_conversation_id", &payload.conversation_id)?;
        if let Some(cost) = payload.cost {
            ph_event.insert_prop("$ai_cost", cost)?;
        }
    }

    Ok(ph_event)
}

#[async_trait::async_trait]
impl Collect for Tracker {
    async fn collect(&self, event: Event) -> Result<()> {
        let ph_event = build_ph_event(&event)?;
        let client = self.get_client().await;
        client.capture(ph_event).await?;
        Ok(())
    }
}
