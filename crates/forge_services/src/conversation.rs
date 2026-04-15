use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use forge_app::ConversationService;
use forge_app::domain::{Conversation, ConversationId};
use forge_domain::ConversationRepository;
use tokio::sync::Mutex;

/// Service for managing conversations with serialized database writes to
/// prevent SQLite contention. SQLite only allows one writer at a time, so all
/// write operations are serialized at the service layer to prevent pool
/// exhaustion.
#[derive(Clone)]
pub struct ForgeConversationService<S> {
    conversation_repository: Arc<S>,
    /// Global write lock to serialize all database writes.
    /// SQLite only allows one writer at a time, so we queue all writes
    /// to prevent pool contention when multiple tasks try to write
    /// concurrently.
    write_lock: Arc<Mutex<()>>,
}

impl<S: ConversationRepository> ForgeConversationService<S> {
    /// Creates a new ForgeConversationService with the provided repository
    pub fn new(repo: Arc<S>) -> Self {
        Self {
            conversation_repository: repo,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Runs a write operation serialized behind a global lock.
    ///
    /// This prevents multiple concurrent writes from exhausting the connection
    /// pool while waiting for SQLite's single writer lock.
    async fn run_serialized_write<F, Fut, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        let _guard = self.write_lock.lock().await;
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
        self.run_serialized_write(|| async {
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
        self.run_serialized_write(|| async {
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
        self.run_serialized_write(|| async {
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
    async fn test_global_write_serialization_prevents_overlapping_writes() -> Result<()> {
        // Test that ALL writes are serialized, not just same-conversation writes
        // This is critical because SQLite only allows one writer at a time
        let repository = Arc::new(RecordingConversationRepository::new());
        let service = Arc::new(ForgeConversationService::new(repository.clone()));

        let conversation1 =
            Conversation::new(ConversationId::generate()).title("First".to_string());
        let conversation2 =
            Conversation::new(ConversationId::generate()).title("Second".to_string());

        // Spawn two tasks writing DIFFERENT conversations concurrently
        let service1 = service.clone();
        let task1 = tokio::spawn(async move { service1.upsert_conversation(conversation1).await });

        let service2 = service.clone();
        let task2 = tokio::spawn(async move { service2.upsert_conversation(conversation2).await });

        task1.await??;
        task2.await??;

        // With global serialization, no two writes should overlap
        let actual = repository.overlapping_upserts().await;
        let expected = Vec::<ConversationId>::new();
        assert_eq!(
            actual, expected,
            "Different-conversation writes should not overlap"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_same_conversation_writes_serialized() -> Result<()> {
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
