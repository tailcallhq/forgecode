use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use forge_domain::{
    Conversation, ConversationId, EndPayload, EventData, EventHandle, StartPayload,
};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::agent::AgentService;
use crate::title_generator::TitleGenerator;

/// Per-conversation title generation state.
struct TitleGenerationState {
    rx: oneshot::Receiver<Option<String>>,
    handle: JoinHandle<()>,
}

/// Hook handler that generates a conversation title asynchronously.
#[derive(Clone)]
pub struct TitleGenerationHandler<S> {
    services: Arc<S>,
    title_tasks: Arc<DashMap<ConversationId, TitleGenerationState>>,
    /// When false, `handle()` returns immediately without spawning a
    /// background turn. Lets callers (e.g. one-shot `--prompt` runs) skip
    /// the parallel LLM call that title generation would otherwise add.
    enabled: bool,
}

impl<S> TitleGenerationHandler<S> {
    /// Creates a new title generation handler with title generation enabled.
    pub fn new(services: Arc<S>) -> Self {
        Self::with_enabled(services, true)
    }

    /// Creates a handler whose `enabled` flag controls whether title
    /// generation actually runs. A disabled handler is registered just like
    /// an enabled one but short-circuits the LLM call.
    pub fn with_enabled(services: Arc<S>, enabled: bool) -> Self {
        Self {
            services,
            title_tasks: Arc::new(DashMap::new()),
            enabled,
        }
    }
}

#[async_trait]
impl<S: AgentService> EventHandle<EventData<StartPayload>> for TitleGenerationHandler<S> {
    async fn handle(
        &self,
        event: &EventData<StartPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        if conversation.title.is_some() {
            return Ok(());
        }

        let user_prompt = conversation
            .context
            .as_ref()
            .and_then(|c| {
                c.messages
                    .iter()
                    .find(|m| m.has_role(forge_domain::Role::User))
            })
            .and_then(|e| e.message.as_value())
            .and_then(|e| e.as_user_prompt());

        let Some(user_prompt) = user_prompt else {
            return Ok(());
        };

        let generator = TitleGenerator::new(
            self.services.clone(),
            user_prompt.clone(),
            event.model_id.clone(),
            Some(event.agent.provider.clone()),
        )
        .reasoning(event.agent.reasoning.clone());

        // `or_insert_with` holds the shard lock for its entire call. Any occupied
        // entry — InProgress, Awaiting, or Done — is left untouched, so at most
        // one task is ever spawned per conversation id.
        self.title_tasks.entry(conversation.id).or_insert_with(|| {
            let (tx, rx) = oneshot::channel();
            let handle = tokio::spawn(async move {
                let title = generator.generate().await.ok().flatten();
                let _ = tx.send(title);
            });
            TitleGenerationState { rx, handle }
        });

        Ok(())
    }
}

#[async_trait]
impl<S: AgentService> EventHandle<EventData<EndPayload>> for TitleGenerationHandler<S> {
    async fn handle(
        &self,
        _event: &EventData<EndPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        if let Some((_, entry)) = self.title_tasks.remove(&conversation.id) {
            let handle = &entry.handle;
            let rx = entry.rx;

            if rx.is_empty() {
                handle.abort();
            } else if let Some(title) = rx.await? {
                conversation.title = Some(title);
            }
        }

        Ok(())
    }
}

impl<S> Drop for TitleGenerationHandler<S> {
    fn drop(&mut self) {
        // Explicitly abort every spawned task before clearing the map.
        // Dropping a `JoinHandle` does *not* abort the underlying Tokio task —
        // the task would keep running until completion. Calling `.abort()`
        // ensures the tasks are cancelled immediately so the runtime can
        // shut down cleanly without waiting for pending LLM calls.
        for entry in self.title_tasks.iter() {
            entry.handle.abort();
        }
        self.title_tasks.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use forge_domain::{
        Agent, ChatCompletionMessage, Context, ContextMessage, Conversation, EventValue, ModelId,
        ProviderId, Role, TextMessage, ToolCallContext, ToolCallFull, ToolResult,
    };
    use pretty_assertions::assert_eq;

    use super::*;

    #[derive(Clone)]
    struct MockAgentService;

    #[async_trait]
    impl AgentService for MockAgentService {
        async fn chat_agent(
            &self,
            _id: &ModelId,
            _context: Context,
            _provider_id: Option<ProviderId>,
        ) -> forge_domain::ResultStream<ChatCompletionMessage, anyhow::Error> {
            Ok(Box::pin(futures::stream::empty()))
        }

        async fn call(
            &self,
            _agent: &Agent,
            _context: &ToolCallContext,
            _call: ToolCallFull,
        ) -> ToolResult {
            unreachable!("Not used in tests")
        }

        async fn update(&self, _conversation: Conversation) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn setup(message: &str) -> (TitleGenerationHandler<MockAgentService>, Conversation) {
        let handler = TitleGenerationHandler::new(Arc::new(MockAgentService));
        let context = Context::default().add_message(ContextMessage::Text(
            TextMessage::new(Role::User, message).raw_content(EventValue::text(message)),
        ));
        let conversation = Conversation::generate().context(context);
        (handler, conversation)
    }

    fn event<T: Send + Sync>(payload: T) -> EventData<T> {
        EventData::new(
            Agent::new("t", "t".to_string().into(), ModelId::new("t")),
            ModelId::new("t"),
            payload,
        )
    }

    #[tokio::test]
    async fn test_start_skips_if_title_exists() {
        let (handler, mut conversation) = setup("test message");
        conversation.title = Some("existing".into());

        handler
            .handle(&event(StartPayload), &mut conversation)
            .await
            .unwrap();

        assert!(!handler.title_tasks.contains_key(&conversation.id));
    }

    #[tokio::test]
    async fn test_start_skips_if_task_already_in_progress() {
        let (handler, mut conversation) = setup("test message");
        let (tx, rx) = oneshot::channel();
        tx.send(Some("original".to_string())).unwrap();
        let handle = tokio::spawn(async {});
        handle.abort();
        handler
            .title_tasks
            .insert(conversation.id, TitleGenerationState { rx, handle });

        handler
            .handle(&event(StartPayload), &mut conversation)
            .await
            .unwrap();

        // Entry should still exist (wasn't replaced)
        assert!(handler.title_tasks.contains_key(&conversation.id));
    }

    #[tokio::test]
    async fn test_end_sets_title_from_completed_task() {
        let (handler, mut conversation) = setup("test message");
        let (tx, rx) = oneshot::channel();
        tx.send(Some("generated".to_string())).unwrap();
        let handle = tokio::spawn(async {});
        handle.abort();
        handler
            .title_tasks
            .insert(conversation.id, TitleGenerationState { rx, handle });

        handler
            .handle(&event(EndPayload), &mut conversation)
            .await
            .unwrap();

        assert_eq!(conversation.title, Some("generated".into()));
        // Entry should be removed after successful title generation
        assert!(!handler.title_tasks.contains_key(&conversation.id));
    }

    #[tokio::test]
    async fn test_end_handles_task_cancellation() {
        let (handler, mut conversation) = setup("test message");
        let (tx, rx) = oneshot::channel::<Option<String>>();
        // Drop the sender to simulate a cancelled task.
        drop(tx);
        let handle = tokio::spawn(async {});
        handle.abort();
        handler
            .title_tasks
            .insert(conversation.id, TitleGenerationState { rx, handle });

        handler
            .handle(&event(EndPayload), &mut conversation)
            .await
            .unwrap();

        assert!(conversation.title.is_none());
        assert!(!handler.title_tasks.contains_key(&conversation.id));
    }

    /// When EndPayload is received, the spawned task should be aborted so it
    /// doesn't continue running unnecessarily.
    #[tokio::test]
    async fn test_end_aborts_in_progress_task() {
        let (handler, mut conversation) = setup("test message");
        let (tx, rx) = oneshot::channel::<Option<String>>();
        let handle = tokio::spawn(async move {
            // Simulate a long-running task that would block indefinitely.
            tokio::time::sleep(Duration::from_secs(60)).await;
            let _ = tx.send(None);
        });

        handler
            .title_tasks
            .insert(conversation.id, TitleGenerationState { rx, handle });

        handler
            .handle(&event(EndPayload), &mut conversation)
            .await
            .unwrap();

        // Entry should have been removed from map
        assert!(!handler.title_tasks.contains_key(&conversation.id));

        // Verify the task is no longer running by checking that the
        // EndPayload handler didn't hang (it completed immediately).
        assert!(conversation.title.is_none());
    }

    /// Many concurrent StartPayload calls for the same conversation id must
    /// result in exactly one spawned task.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_start_spawns_only_one_task() {
        let (handler, conversation) = setup("test message");
        let barrier = Arc::new(tokio::sync::Barrier::new(20));
        let handler = Arc::new(handler);

        let mut joins = Vec::new();
        for _ in 0..20 {
            let handler = handler.clone();
            let barrier = barrier.clone();
            let mut conv = conversation.clone();
            joins.push(tokio::spawn(async move {
                barrier.wait().await;
                handler
                    .handle(&event(StartPayload), &mut conv)
                    .await
                    .unwrap();
            }));
        }
        for j in joins {
            j.await.unwrap();
        }

        // Only one task should exist in the map
        assert_eq!(handler.title_tasks.len(), 1);
    }
}
