use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use dashmap::DashMap;
use forge_app::ConversationService;
use forge_app::domain::{Conversation, ConversationId};
use forge_domain::ConversationRepository;
use tokio::sync::Mutex;

/// Service for managing conversations, including creation, retrieval, and
/// updates
#[derive(Clone)]
pub struct ForgeConversationService<S> {
    conversation_repository: Arc<S>,
    conversation_lock_cache: Arc<DashMap<ConversationId, Arc<Mutex<()>>>>,
}

impl<S: ConversationRepository> ForgeConversationService<S> {
    /// Creates a new ForgeConversationService with the provided repository
    pub fn new(repo: Arc<S>) -> Self {
        Self {
            conversation_repository: repo,
            conversation_lock_cache: Arc::new(DashMap::new()),
        }
    }

    async fn run_serialized_write<F, Fut, T>(
        &self,
        conversation_id: ConversationId,
        operation: F,
    ) -> Result<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        let conversation_lock = self
            .conversation_lock_cache
            .entry(conversation_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = conversation_lock.lock().await;
        operation().await
    }
}

#[async_trait::async_trait]
impl<S: ConversationRepository> ConversationService for ForgeConversationService<S> {
    async fn modify_conversation<F, T>(&self, id: &ConversationId, f: F) -> Result<T>
    where
        F: FnOnce(&mut Conversation) -> T + Send,
        T: Send,
    {
        self.run_serialized_write(*id, || async {
            let mut conversation = self
                .conversation_repository
                .get_conversation(id)
                .await?
                .ok_or_else(|| forge_app::domain::Error::ConversationNotFound(*id))?;
            let out = f(&mut conversation);
            self.conversation_repository
                .upsert_conversation(conversation)
                .await?;
            Ok(out)
        })
        .await
    }

    async fn find_conversation(&self, id: &ConversationId) -> Result<Option<Conversation>> {
        self.conversation_repository.get_conversation(id).await
    }

    async fn upsert_conversation(&self, conversation: Conversation) -> Result<()> {
        self.run_serialized_write(conversation.id, || async {
            self.conversation_repository
                .upsert_conversation(conversation)
                .await?;
            Ok(())
        })
        .await
    }

    async fn get_conversations(&self, limit: Option<usize>) -> Result<Option<Vec<Conversation>>> {
        self.conversation_repository
            .get_all_conversations(limit)
            .await
    }

    async fn last_conversation(&self) -> Result<Option<Conversation>> {
        self.conversation_repository.get_last_conversation().await
    }

    async fn delete_conversation(&self, conversation_id: &ConversationId) -> Result<()> {
        self.run_serialized_write(*conversation_id, || async {
            self.conversation_repository
                .delete_conversation(conversation_id)
                .await
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use tokio::sync::Mutex;

    use super::*;

    struct RecordingConversationRepository {
        conversations: Mutex<HashMap<ConversationId, Conversation>>,
        active_upserts: Mutex<HashSet<ConversationId>>,
        overlapping_upserts: Mutex<Vec<ConversationId>>,
    }

    impl RecordingConversationRepository {
        fn new() -> Self {
            Self {
                conversations: Mutex::new(HashMap::new()),
                active_upserts: Mutex::new(HashSet::new()),
                overlapping_upserts: Mutex::new(Vec::new()),
            }
        }

        async fn overlapping_upserts(&self) -> Vec<ConversationId> {
            self.overlapping_upserts.lock().await.clone()
        }
    }

    #[async_trait::async_trait]
    impl ConversationRepository for RecordingConversationRepository {
        async fn upsert_conversation(&self, conversation: Conversation) -> Result<()> {
            {
                let mut active_upserts = self.active_upserts.lock().await;
                if !active_upserts.insert(conversation.id) {
                    self.overlapping_upserts.lock().await.push(conversation.id);
                }
            }

            tokio::time::sleep(Duration::from_millis(50)).await;

            self.conversations
                .lock()
                .await
                .insert(conversation.id, conversation.clone());

            self.active_upserts.lock().await.remove(&conversation.id);
            Ok(())
        }

        async fn get_conversation(&self, id: &ConversationId) -> Result<Option<Conversation>> {
            Ok(self.conversations.lock().await.get(id).cloned())
        }

        async fn get_all_conversations(
            &self,
            _limit: Option<usize>,
        ) -> Result<Option<Vec<Conversation>>> {
            let actual = self
                .conversations
                .lock()
                .await
                .values()
                .cloned()
                .collect::<Vec<_>>();
            if actual.is_empty() {
                Ok(None)
            } else {
                Ok(Some(actual))
            }
        }

        async fn get_last_conversation(&self) -> Result<Option<Conversation>> {
            Ok(self.conversations.lock().await.values().last().cloned())
        }

        async fn delete_conversation(&self, conversation_id: &ConversationId) -> Result<()> {
            self.conversations.lock().await.remove(conversation_id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_upsert_conversation_serializes_same_conversation_id() -> Result<()> {
        let fixture = Conversation::new(ConversationId::generate()).title("Serialized".to_string());
        let repository = Arc::new(RecordingConversationRepository::new());
        let service = Arc::new(ForgeConversationService::new(repository.clone()));

        let first_service = service.clone();
        let first_fixture = fixture.clone();
        let first_task =
            tokio::spawn(async move { first_service.upsert_conversation(first_fixture).await });

        let second_service = service.clone();
        let second_fixture = fixture.clone();
        let second_task =
            tokio::spawn(async move { second_service.upsert_conversation(second_fixture).await });

        first_task.await??;
        second_task.await??;

        let actual = repository.overlapping_upserts().await;
        let expected = Vec::<ConversationId>::new();
        assert_eq!(actual, expected);
        Ok(())
    }
}
