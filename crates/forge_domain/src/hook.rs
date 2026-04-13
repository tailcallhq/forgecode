use async_trait::async_trait;
use derive_more::From;
use derive_setters::Setters;

use crate::{Agent, ChatCompletionMessageFull, Conversation, ModelId, ToolCallFull, ToolResult};

/// A container for lifecycle events with agent and model ID context
///
/// This struct provides a consistent structure for all lifecycle events,
/// containing the agent and model ID along with event-specific payload data.
#[derive(Debug, PartialEq, Clone)]
pub struct EventData<P: Send + Sync> {
    /// The agent associated with this event
    pub agent: Agent,
    /// The model ID being used
    pub model_id: ModelId,
    /// Event-specific payload data
    pub payload: P,
}

impl<P: Send + Sync> EventData<P> {
    /// Creates a new event with the given agent, model ID, and payload
    pub fn new(agent: Agent, model_id: ModelId, payload: P) -> Self {
        Self { agent, model_id, payload }
    }
}

/// Payload for the Start event
#[derive(Debug, PartialEq, Clone, Default)]
pub struct StartPayload;

/// Payload for the End event
#[derive(Debug, PartialEq, Clone, Default)]
pub struct EndPayload;

/// Payload for the Request event
#[derive(Debug, PartialEq, Clone, Setters)]
#[setters(into)]
pub struct RequestPayload {
    /// The number of requests made
    pub request_count: usize,
}

impl RequestPayload {
    /// Creates a new request payload
    pub fn new(request_count: usize) -> Self {
        Self { request_count }
    }
}

/// Payload for the Response event
#[derive(Debug, PartialEq, Clone, Setters)]
#[setters(into)]
pub struct ResponsePayload {
    /// The full response message from the LLM
    pub message: ChatCompletionMessageFull,
}

impl ResponsePayload {
    /// Creates a new response payload
    pub fn new(message: ChatCompletionMessageFull) -> Self {
        Self { message }
    }
}

/// Payload for the ToolcallStart event
#[derive(Debug, PartialEq, Clone, Setters)]
#[setters(into)]
pub struct ToolcallStartPayload {
    /// The tool call details
    pub tool_call: ToolCallFull,
}

impl ToolcallStartPayload {
    /// Creates a new tool call start payload
    pub fn new(tool_call: ToolCallFull) -> Self {
        Self { tool_call }
    }
}

/// Payload for the ToolcallEnd event
#[derive(Debug, PartialEq, Clone, Setters)]
#[setters(into)]
pub struct ToolcallEndPayload {
    /// The original tool call that was executed
    pub tool_call: ToolCallFull,
    /// The tool result (success or failure)
    pub result: ToolResult,
}

impl ToolcallEndPayload {
    /// Creates a new tool call end payload
    pub fn new(tool_call: ToolCallFull, result: ToolResult) -> Self {
        Self { tool_call, result }
    }
}

/// Lifecycle events that can occur during conversation processing
#[derive(Debug, PartialEq, Clone, From)]
pub enum LifecycleEvent {
    /// Event fired when conversation processing starts
    Start(EventData<StartPayload>),

    /// Event fired when conversation processing ends
    End(EventData<EndPayload>),

    /// Event fired when a request is made to the LLM
    Request(EventData<RequestPayload>),

    /// Event fired when a response is received from the LLM
    Response(EventData<ResponsePayload>),

    /// Event fired when a tool call starts
    ToolcallStart(EventData<ToolcallStartPayload>),

    /// Event fired when a tool call ends
    ToolcallEnd(EventData<ToolcallEndPayload>),
}

/// Trait for handling lifecycle events
///
/// Implementations of this trait can be used to react to different
/// stages of conversation processing.
#[async_trait]
pub trait EventHandle<T: Send + Sync>: Send + Sync {
    /// Handles a lifecycle event and potentially modifies the conversation
    ///
    /// # Arguments
    /// * `event` - The lifecycle event that occurred
    /// * `conversation` - The current conversation state (mutable)
    ///
    /// # Errors
    /// Returns an error if the event handling fails
    async fn handle(&self, event: &T, conversation: &mut Conversation) -> anyhow::Result<()>;
}

/// Extension trait for combining event handlers
///
/// This trait provides methods to combine multiple event handlers into a single
/// handler that executes them in sequence.
pub trait EventHandleExt<T: Send + Sync>: EventHandle<T> {
    /// Combines this handler with another handler, creating a new handler that
    /// runs both in sequence
    ///
    /// When an event is handled, both handlers run in sequence.
    ///
    /// # Arguments
    /// * `other` - Another handler to combine with this one
    ///
    /// # Returns
    /// A new boxed handler that combines both handlers
    fn and<H: EventHandle<T> + 'static>(self, other: H) -> Box<dyn EventHandle<T>>
    where
        Self: Sized + 'static;
}

impl<T: Send + Sync + 'static, A: EventHandle<T> + 'static> EventHandleExt<T> for A {
    fn and<H: EventHandle<T> + 'static>(self, other: H) -> Box<dyn EventHandle<T>>
    where
        Self: Sized + 'static,
    {
        Box::new(CombinedHandler(Box::new(self), Box::new(other)))
    }
}

// Implement EventHandle for Box<dyn EventHandle> to allow using boxed handlers
#[async_trait]
impl<T: Send + Sync> EventHandle<T> for Box<dyn EventHandle<T>> {
    async fn handle(&self, event: &T, conversation: &mut Conversation) -> anyhow::Result<()> {
        (**self).handle(event, conversation).await
    }
}

/// A hook that contains handlers for all lifecycle events
///
/// Hooks allow you to attach custom behavior at specific points
/// during conversation processing.
pub struct Hook {
    on_start: Box<dyn EventHandle<EventData<StartPayload>>>,
    on_end: Box<dyn EventHandle<EventData<EndPayload>>>,
    on_request: Box<dyn EventHandle<EventData<RequestPayload>>>,
    on_response: Box<dyn EventHandle<EventData<ResponsePayload>>>,
    on_toolcall_start: Box<dyn EventHandle<EventData<ToolcallStartPayload>>>,
    on_toolcall_end: Box<dyn EventHandle<EventData<ToolcallEndPayload>>>,
}

impl Default for Hook {
    fn default() -> Self {
        Self {
            on_start: Box::new(NoOpHandler),
            on_end: Box::new(NoOpHandler),
            on_request: Box::new(NoOpHandler),
            on_response: Box::new(NoOpHandler),
            on_toolcall_start: Box::new(NoOpHandler),
            on_toolcall_end: Box::new(NoOpHandler),
        }
    }
}

impl Hook {
    /// Creates a new hook with custom handlers for all event types
    ///
    /// # Arguments
    /// * `on_start` - Handler for start events
    /// * `on_end` - Handler for end events
    /// * `on_request` - Handler for request events
    /// * `on_response` - Handler for response events
    /// * `on_toolcall_start` - Handler for tool call start events
    /// * `on_toolcall_end` - Handler for tool call end events
    pub fn new(
        on_start: impl Into<Box<dyn EventHandle<EventData<StartPayload>>>>,
        on_end: impl Into<Box<dyn EventHandle<EventData<EndPayload>>>>,
        on_request: impl Into<Box<dyn EventHandle<EventData<RequestPayload>>>>,
        on_response: impl Into<Box<dyn EventHandle<EventData<ResponsePayload>>>>,
        on_toolcall_start: impl Into<Box<dyn EventHandle<EventData<ToolcallStartPayload>>>>,
        on_toolcall_end: impl Into<Box<dyn EventHandle<EventData<ToolcallEndPayload>>>>,
    ) -> Self {
        Self {
            on_start: on_start.into(),
            on_end: on_end.into(),
            on_request: on_request.into(),
            on_response: on_response.into(),
            on_toolcall_start: on_toolcall_start.into(),
            on_toolcall_end: on_toolcall_end.into(),
        }
    }
}

impl Hook {
    /// Sets the start event handler
    ///
    /// # Arguments
    /// * `handler` - Handler for start events (automatically boxed)
    pub fn on_start(
        mut self,
        handler: impl EventHandle<EventData<StartPayload>> + 'static,
    ) -> Self {
        self.on_start = Box::new(handler);
        self
    }

    /// Sets the end event handler
    ///
    /// # Arguments
    /// * `handler` - Handler for end events (automatically boxed)
    pub fn on_end(mut self, handler: impl EventHandle<EventData<EndPayload>> + 'static) -> Self {
        self.on_end = Box::new(handler);
        self
    }

    /// Sets the request event handler
    ///
    /// # Arguments
    /// * `handler` - Handler for request events (automatically boxed)
    pub fn on_request(
        mut self,
        handler: impl EventHandle<EventData<RequestPayload>> + 'static,
    ) -> Self {
        self.on_request = Box::new(handler);
        self
    }

    /// Sets the response event handler
    ///
    /// # Arguments
    /// * `handler` - Handler for response events (automatically boxed)
    pub fn on_response(
        mut self,
        handler: impl EventHandle<EventData<ResponsePayload>> + 'static,
    ) -> Self {
        self.on_response = Box::new(handler);
        self
    }

    /// Sets the tool call start event handler
    ///
    /// # Arguments
    /// * `handler` - Handler for tool call start events (automatically boxed)
    pub fn on_toolcall_start(
        mut self,
        handler: impl EventHandle<EventData<ToolcallStartPayload>> + 'static,
    ) -> Self {
        self.on_toolcall_start = Box::new(handler);
        self
    }

    /// Sets the tool call end event handler
    ///
    /// # Arguments
    /// * `handler` - Handler for tool call end events (automatically boxed)
    pub fn on_toolcall_end(
        mut self,
        handler: impl EventHandle<EventData<ToolcallEndPayload>> + 'static,
    ) -> Self {
        self.on_toolcall_end = Box::new(handler);
        self
    }
}

impl Hook {
    /// Combines this hook with another hook, creating a new hook that runs both
    /// handlers in sequence
    ///
    /// When an event is handled, the first hook's handler runs first, then the
    /// second hook's handler runs.
    ///
    /// # Arguments
    /// * `other` - Another hook to combine with this one
    ///
    /// # Returns
    /// A new hook that combines both hooks' handlers
    pub fn zip(self, other: Hook) -> Self {
        Self {
            on_start: self.on_start.and(other.on_start),
            on_end: self.on_end.and(other.on_end),
            on_request: self.on_request.and(other.on_request),
            on_response: self.on_response.and(other.on_response),
            on_toolcall_start: self.on_toolcall_start.and(other.on_toolcall_start),
            on_toolcall_end: self.on_toolcall_end.and(other.on_toolcall_end),
        }
    }
}

// Implement EventHandle for Hook to allow hooks to handle LifecycleEvent
#[async_trait]
impl EventHandle<LifecycleEvent> for Hook {
    async fn handle(
        &self,
        event: &LifecycleEvent,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        match &event {
            LifecycleEvent::Start(data) => self.on_start.handle(data, conversation).await,
            LifecycleEvent::End(data) => self.on_end.handle(data, conversation).await,
            LifecycleEvent::Request(data) => self.on_request.handle(data, conversation).await,
            LifecycleEvent::Response(data) => self.on_response.handle(data, conversation).await,
            LifecycleEvent::ToolcallStart(data) => {
                self.on_toolcall_start.handle(data, conversation).await
            }
            LifecycleEvent::ToolcallEnd(data) => {
                self.on_toolcall_end.handle(data, conversation).await
            }
        }
    }
}

/// A handler that combines two event handlers with sequential execution
///
/// Runs the first handler, then runs the second handler.
///
/// This is used internally by the `Hook::zip` and `EventHandleExt::and`
/// methods.
struct CombinedHandler<T: Send + Sync>(Box<dyn EventHandle<T>>, Box<dyn EventHandle<T>>);

#[async_trait]
impl<T: Send + Sync> EventHandle<T> for CombinedHandler<T> {
    async fn handle(&self, event: &T, conversation: &mut Conversation) -> anyhow::Result<()> {
        // Run the first handler
        self.0.handle(event, conversation).await?;
        // Run the second handler with the cloned event
        self.1.handle(event, conversation).await
    }
}

/// A no-op handler that does nothing
///
/// This is useful as a default handler when you only want to
/// handle specific events.
#[derive(Debug, Default)]
pub struct NoOpHandler;

#[async_trait]
impl<T: Send + Sync> EventHandle<T> for NoOpHandler {
    async fn handle(&self, _: &T, _: &mut Conversation) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl<T: Send + Sync, F, Fut> EventHandle<T> for F
where
    F: Fn(&T, &mut Conversation) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = anyhow::Result<()>> + Send,
{
    async fn handle(&self, event: &T, conversation: &mut Conversation) -> anyhow::Result<()> {
        (self)(event, conversation).await
    }
}

impl<T: Send + Sync, F, Fut> From<F> for Box<dyn EventHandle<T>>
where
    F: Fn(&T, &mut Conversation) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    fn from(handler: F) -> Self {
        Box::new(handler)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{Agent, AgentId, Conversation, ModelId, ProviderId};

    fn test_agent() -> Agent {
        Agent::new(
            AgentId::new("test_agent"),
            ProviderId::FORGE,
            ModelId::new("test-model"),
        )
    }

    fn test_model_id() -> ModelId {
        ModelId::new("test-model")
    }

    #[test]
    fn test_no_op_handler() {
        let handler = NoOpHandler;
        let conversation = Conversation::generate();

        // This test just ensures NoOpHandler compiles and is constructible
        let _ = handler;
        let _ = conversation;
    }

    #[tokio::test]
    async fn test_hook_on_start() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let events_clone = events.clone();

        let hook = Hook::default().on_start(
            move |event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let events = events_clone.clone();
                let event = event.clone();
                async move {
                    events.lock().unwrap().push(event);
                    Ok(())
                }
            },
        );

        let mut conversation = Conversation::generate();

        hook.handle(
            &LifecycleEvent::Start(EventData::new(test_agent(), test_model_id(), StartPayload)),
            &mut conversation,
        )
        .await
        .unwrap();

        let handled = events.lock().unwrap();
        assert_eq!(handled.len(), 1);
        assert_eq!(
            handled[0],
            EventData::new(test_agent(), test_model_id(), StartPayload)
        );
    }

    #[tokio::test]
    async fn test_hook_builder() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let hook = Hook::default()
            .on_start({
                let events = events.clone();
                move |event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                    let events = events.clone();
                    let event = LifecycleEvent::Start(event.clone());
                    async move {
                        events.lock().unwrap().push(event);
                        Ok(())
                    }
                }
            })
            .on_end({
                let events = events.clone();
                move |event: &EventData<EndPayload>, _conversation: &mut Conversation| {
                    let events = events.clone();
                    let event = LifecycleEvent::End(event.clone());
                    async move {
                        events.lock().unwrap().push(event);
                        Ok(())
                    }
                }
            })
            .on_request({
                let events = events.clone();
                move |event: &EventData<RequestPayload>, _conversation: &mut Conversation| {
                    let events = events.clone();
                    let event = LifecycleEvent::Request(event.clone());
                    async move {
                        events.lock().unwrap().push(event);
                        Ok(())
                    }
                }
            });

        let mut conversation = Conversation::generate();

        // Test Start event
        hook.handle(
            &LifecycleEvent::Start(EventData::new(test_agent(), test_model_id(), StartPayload)),
            &mut conversation,
        )
        .await
        .unwrap();
        // Test End event
        hook.handle(
            &LifecycleEvent::End(EventData::new(test_agent(), test_model_id(), EndPayload)),
            &mut conversation,
        )
        .await
        .unwrap();
        // Test Request event
        hook.handle(
            &LifecycleEvent::Request(EventData::new(
                test_agent(),
                test_model_id(),
                RequestPayload::new(1),
            )),
            &mut conversation,
        )
        .await
        .unwrap();

        let handled = events.lock().unwrap();
        assert_eq!(handled.len(), 3);
        assert_eq!(
            handled[0],
            LifecycleEvent::Start(EventData::new(test_agent(), test_model_id(), StartPayload))
        );
        assert_eq!(
            handled[1],
            LifecycleEvent::End(EventData::new(test_agent(), test_model_id(), EndPayload))
        );
        assert_eq!(
            handled[2],
            LifecycleEvent::Request(EventData::new(
                test_agent(),
                test_model_id(),
                RequestPayload::new(1)
            ))
        );
    }

    #[tokio::test]
    async fn test_hook_all_events() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let hook = Hook::new(
            {
                let events = events.clone();
                move |event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                    let events = events.clone();
                    let event = LifecycleEvent::Start(event.clone());
                    async move {
                        events.lock().unwrap().push(event);
                        Ok(())
                    }
                }
            },
            {
                let events = events.clone();
                move |event: &EventData<EndPayload>, _conversation: &mut Conversation| {
                    let events = events.clone();
                    let event = LifecycleEvent::End(event.clone());
                    async move {
                        events.lock().unwrap().push(event);
                        Ok(())
                    }
                }
            },
            {
                let events = events.clone();
                move |event: &EventData<RequestPayload>, _conversation: &mut Conversation| {
                    let events = events.clone();
                    let event = LifecycleEvent::Request(event.clone());
                    async move {
                        events.lock().unwrap().push(event);
                        Ok(())
                    }
                }
            },
            {
                let events = events.clone();
                move |event: &EventData<ResponsePayload>, _conversation: &mut Conversation| {
                    let events = events.clone();
                    let event = LifecycleEvent::Response(event.clone());
                    async move {
                        events.lock().unwrap().push(event);
                        Ok(())
                    }
                }
            },
            {
                let events = events.clone();
                move |event: &EventData<ToolcallStartPayload>, _conversation: &mut Conversation| {
                    let events = events.clone();
                    let event = LifecycleEvent::ToolcallStart(event.clone());
                    async move {
                        events.lock().unwrap().push(event);
                        Ok(())
                    }
                }
            },
            {
                let events = events.clone();
                move |event: &EventData<ToolcallEndPayload>, _conversation: &mut Conversation| {
                    let events = events.clone();
                    let event = LifecycleEvent::ToolcallEnd(event.clone());
                    async move {
                        events.lock().unwrap().push(event);
                        Ok(())
                    }
                }
            },
        );

        let mut conversation = Conversation::generate();

        let all_events = vec![
            LifecycleEvent::Start(EventData::new(test_agent(), test_model_id(), StartPayload)),
            LifecycleEvent::End(EventData::new(test_agent(), test_model_id(), EndPayload)),
            LifecycleEvent::Request(EventData::new(
                test_agent(),
                test_model_id(),
                RequestPayload::new(1),
            )),
            LifecycleEvent::Response(EventData::new(
                test_agent(),
                test_model_id(),
                ResponsePayload::new(ChatCompletionMessageFull {
                    content: "test".to_string(),
                    reasoning: None,
                    tool_calls: vec![],
                    thought_signature: None,
                    reasoning_details: None,
                    usage: crate::Usage::default(),
                    finish_reason: None,
                    phase: None,
                }),
            )),
            LifecycleEvent::ToolcallStart(EventData::new(
                test_agent(),
                test_model_id(),
                ToolcallStartPayload::new(ToolCallFull::new("test_tool")),
            )),
            LifecycleEvent::ToolcallEnd(EventData::new(
                test_agent(),
                test_model_id(),
                ToolcallEndPayload::new(
                    ToolCallFull::new("test_tool"),
                    ToolResult::new("test_tool"),
                ),
            )),
        ];

        for event in all_events {
            hook.handle(&event, &mut conversation).await.unwrap();
        }

        let handled = events.lock().unwrap();
        assert_eq!(handled.len(), 6);
    }

    #[tokio::test]
    async fn test_step_mutable_conversation() {
        let title = std::sync::Arc::new(std::sync::Mutex::new(None));
        let hook = Hook::default().on_start({
            let title = title.clone();
            move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let title = title.clone();
                async move {
                    *title.lock().unwrap() = Some("Modified title".to_string());
                    Ok(())
                }
            }
        });
        let mut conversation = Conversation::generate();

        assert!(title.lock().unwrap().is_none());

        hook.handle(
            &LifecycleEvent::Start(EventData::new(test_agent(), test_model_id(), StartPayload)),
            &mut conversation,
        )
        .await
        .unwrap();

        assert_eq!(*title.lock().unwrap(), Some("Modified title".to_string()));
    }

    #[test]
    fn test_hook_default() {
        let hook = Hook::default();

        // Just ensure it compiles and is constructible
        let _ = hook;
    }

    #[tokio::test]
    async fn test_hook_zip() {
        let counter1 = std::sync::Arc::new(std::sync::Mutex::new(0));
        let counter2 = std::sync::Arc::new(std::sync::Mutex::new(0));

        let hook1 = Hook::default().on_start({
            let counter = counter1.clone();
            move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let counter = counter.clone();
                async move {
                    *counter.lock().unwrap() += 1;
                    Ok(())
                }
            }
        });

        let hook2 = Hook::default().on_start({
            let counter = counter2.clone();
            move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let counter = counter.clone();
                async move {
                    *counter.lock().unwrap() += 1;
                    Ok(())
                }
            }
        });
        let combined: Hook = hook1.zip(hook2);

        let mut conversation = Conversation::generate();
        combined
            .handle(
                &LifecycleEvent::Start(EventData::new(test_agent(), test_model_id(), StartPayload)),
                &mut conversation,
            )
            .await
            .unwrap();

        // Both handlers should have been called
        assert_eq!(*counter1.lock().unwrap(), 1);
        assert_eq!(*counter2.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_hook_zip_multiple() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let hook1 = Hook::default().on_start({
            let events = events.clone();
            move |event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let events = events.clone();
                let event = event.clone();
                async move {
                    events.lock().unwrap().push(format!("h1:{:?}", event));
                    Ok(())
                }
            }
        });

        let hook2 = Hook::default().on_start({
            let events = events.clone();
            move |event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let events = events.clone();
                let event = event.clone();
                async move {
                    events.lock().unwrap().push(format!("h2:{:?}", event));
                    Ok(())
                }
            }
        });

        let hook3 = Hook::default().on_start({
            let events = events.clone();
            move |event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let events = events.clone();
                let event = event.clone();
                async move {
                    events.lock().unwrap().push(format!("h3:{:?}", event));
                    Ok(())
                }
            }
        });
        let combined: Hook = hook1.zip(hook2).zip(hook3);

        let mut conversation = Conversation::generate();
        combined
            .handle(
                &LifecycleEvent::Start(EventData::new(test_agent(), test_model_id(), StartPayload)),
                &mut conversation,
            )
            .await
            .unwrap();

        let handled = events.lock().unwrap();
        assert_eq!(handled.len(), 3);
        assert!(handled[0].starts_with("h1:EventData"));
        assert!(handled[1].starts_with("h2:EventData"));
        assert!(handled[2].starts_with("h3:EventData"));
    }

    #[tokio::test]
    async fn test_hook_zip_different_events() {
        let start_title = std::sync::Arc::new(std::sync::Mutex::new(None));
        let end_title = std::sync::Arc::new(std::sync::Mutex::new(None));

        let hook1 = Hook::default()
            .on_start({
                let start_title = start_title.clone();
                move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                    let start_title = start_title.clone();
                    async move {
                        *start_title.lock().unwrap() = Some("Start".to_string());
                        Ok(())
                    }
                }
            })
            .on_end({
                let end_title = end_title.clone();
                move |_event: &EventData<EndPayload>, _conversation: &mut Conversation| {
                    let end_title = end_title.clone();
                    async move {
                        *end_title.lock().unwrap() = Some("End".to_string());
                        Ok(())
                    }
                }
            });
        let hook2 = Hook::default();

        let combined: Hook = hook1.zip(hook2);

        let mut conversation = Conversation::generate();

        // Test Start event
        combined
            .handle(
                &LifecycleEvent::Start(EventData::new(test_agent(), test_model_id(), StartPayload)),
                &mut conversation,
            )
            .await
            .unwrap();
        assert_eq!(*start_title.lock().unwrap(), Some("Start".to_string()));

        // Test End event
        combined
            .handle(
                &LifecycleEvent::End(EventData::new(test_agent(), test_model_id(), EndPayload)),
                &mut conversation,
            )
            .await
            .unwrap();
        assert_eq!(*end_title.lock().unwrap(), Some("End".to_string()));
    }

    #[tokio::test]
    async fn test_event_handle_ext_and() {
        let counter1 = std::sync::Arc::new(std::sync::Mutex::new(0));
        let counter2 = std::sync::Arc::new(std::sync::Mutex::new(0));

        let handler1 = {
            let counter = counter1.clone();
            move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let counter = counter.clone();
                async move {
                    *counter.lock().unwrap() += 1;
                    Ok(())
                }
            }
        };

        let handler2 = {
            let counter = counter2.clone();
            move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let counter = counter.clone();
                async move {
                    *counter.lock().unwrap() += 1;
                    Ok(())
                }
            }
        };

        let combined: Box<dyn EventHandle<EventData<StartPayload>>> = handler1.and(handler2);

        let mut conversation = Conversation::generate();
        combined
            .handle(
                &EventData::new(test_agent(), test_model_id(), StartPayload),
                &mut conversation,
            )
            .await
            .unwrap();

        // Both handlers should have been called
        assert_eq!(*counter1.lock().unwrap(), 1);
        assert_eq!(*counter2.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_event_handle_ext_and_boxed() {
        let counter1 = std::sync::Arc::new(std::sync::Mutex::new(0));
        let counter2 = std::sync::Arc::new(std::sync::Mutex::new(0));

        let handler1 = {
            let counter = counter1.clone();
            move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let counter = counter.clone();
                async move {
                    *counter.lock().unwrap() += 1;
                    Ok(())
                }
            }
        };

        let handler2 = {
            let counter = counter2.clone();
            move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let counter = counter.clone();
                async move {
                    *counter.lock().unwrap() += 1;
                    Ok(())
                }
            }
        };

        let combined: Box<dyn EventHandle<EventData<StartPayload>>> = handler1.and(handler2);

        let mut conversation = Conversation::generate();
        combined
            .handle(
                &EventData::new(test_agent(), test_model_id(), StartPayload),
                &mut conversation,
            )
            .await
            .unwrap();

        // Both handlers should have been called
        assert_eq!(*counter1.lock().unwrap(), 1);
        assert_eq!(*counter2.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_event_handle_ext_chain() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let handler1 = {
            let events = events.clone();
            move |event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let events = events.clone();
                let event = event.clone();
                async move {
                    events.lock().unwrap().push(format!("h1:{:?}", event));
                    Ok(())
                }
            }
        };

        let handler2 = {
            let events = events.clone();
            move |event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let events = events.clone();
                let event = event.clone();
                async move {
                    events.lock().unwrap().push(format!("h2:{:?}", event));
                    Ok(())
                }
            }
        };

        let handler3 = {
            let events = events.clone();
            move |event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let events = events.clone();
                let event = event.clone();
                async move {
                    events.lock().unwrap().push(format!("h3:{:?}", event));
                    Ok(())
                }
            }
        };

        // Chain handlers using and()
        let combined: Box<dyn EventHandle<EventData<StartPayload>>> =
            handler1.and(handler2).and(handler3);

        let mut conversation = Conversation::generate();
        combined
            .handle(
                &EventData::new(test_agent(), test_model_id(), StartPayload),
                &mut conversation,
            )
            .await
            .unwrap();

        let handled = events.lock().unwrap();
        assert_eq!(handled.len(), 3);
        assert!(handled[0].starts_with("h1:EventData"));
        assert!(handled[1].starts_with("h2:EventData"));
        assert!(handled[2].starts_with("h3:EventData"));
    }

    #[tokio::test]
    async fn test_event_handle_ext_with_hook() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let start_title = std::sync::Arc::new(std::sync::Mutex::new(None));

        let start_handler = {
            let start_title = start_title.clone();
            move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let start_title = start_title.clone();
                async move {
                    *start_title.lock().unwrap() = Some("Started".to_string());
                    Ok(())
                }
            }
        };

        let logging_handler = {
            let events = events.clone();
            move |event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let events = events.clone();
                let event = event.clone();
                async move {
                    events.lock().unwrap().push(format!("Event: {:?}", event));
                    Ok(())
                }
            }
        };

        // Combine handlers using extension trait
        let combined_handler: Box<dyn EventHandle<EventData<StartPayload>>> =
            start_handler.and(logging_handler);

        let hook = Hook::default().on_start(combined_handler);

        let mut conversation = Conversation::generate();
        hook.handle(
            &LifecycleEvent::Start(EventData::new(test_agent(), test_model_id(), StartPayload)),
            &mut conversation,
        )
        .await
        .unwrap();

        assert_eq!(events.lock().unwrap().len(), 1);
        assert!(events.lock().unwrap()[0].starts_with("Event: EventData"));
    }

    #[tokio::test]
    async fn test_hook_as_event_handle() {
        let start_title = std::sync::Arc::new(std::sync::Mutex::new(None));
        let end_title = std::sync::Arc::new(std::sync::Mutex::new(None));

        let hook = Hook::default()
            .on_start({
                let start_title = start_title.clone();
                move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                    let start_title = start_title.clone();
                    async move {
                        *start_title.lock().unwrap() = Some("Started".to_string());
                        Ok(())
                    }
                }
            })
            .on_end({
                let end_title = end_title.clone();
                move |_event: &EventData<EndPayload>, _conversation: &mut Conversation| {
                    let end_title = end_title.clone();
                    async move {
                        *end_title.lock().unwrap() = Some("Ended".to_string());
                        Ok(())
                    }
                }
            });

        // Test using handle() directly (EventHandle trait)
        let mut conversation = Conversation::generate();
        hook.handle(
            &LifecycleEvent::Start(EventData::new(test_agent(), test_model_id(), StartPayload)),
            &mut conversation,
        )
        .await
        .unwrap();
        assert_eq!(*start_title.lock().unwrap(), Some("Started".to_string()));

        hook.handle(
            &LifecycleEvent::End(EventData::new(test_agent(), test_model_id(), EndPayload)),
            &mut conversation,
        )
        .await
        .unwrap();
        assert_eq!(*end_title.lock().unwrap(), Some("Ended".to_string()));
    }

    #[tokio::test]
    async fn test_hook_combination_with_and() {
        let hook1_title = std::sync::Arc::new(std::sync::Mutex::new(None));
        let hook2_title = std::sync::Arc::new(std::sync::Mutex::new(None));

        let handler1 = {
            let hook1_title = hook1_title.clone();
            move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let hook1_title = hook1_title.clone();
                async move {
                    *hook1_title.lock().unwrap() = Some("Started".to_string());
                    Ok(())
                }
            }
        };
        let handler2 = {
            let hook2_title = hook2_title.clone();
            move |_event: &EventData<StartPayload>, _conversation: &mut Conversation| {
                let hook2_title = hook2_title.clone();
                async move {
                    *hook2_title.lock().unwrap() = Some("Ended".to_string());
                    Ok(())
                }
            }
        };

        // Combine handlers using and() extension method
        let combined: Box<dyn EventHandle<EventData<StartPayload>>> = handler1.and(handler2);

        let mut conversation = Conversation::generate();
        combined
            .handle(
                &EventData::new(test_agent(), test_model_id(), StartPayload),
                &mut conversation,
            )
            .await
            .unwrap();

        // Both handlers should have been called
        assert_eq!(*hook1_title.lock().unwrap(), Some("Started".to_string()));
        assert_eq!(*hook2_title.lock().unwrap(), Some("Ended".to_string()));
    }
}
