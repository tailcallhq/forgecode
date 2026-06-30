use std::sync::Arc;

use anyhow::Result;
use forge_app::ConversationService;
use forge_app::domain::{Conversation, ConversationId, ConversationSummary};
use forge_domain::ConversationRepository;

/// Service for managing conversations, including creation, retrieval, and
/// updates
#[derive(Clone)]
pub struct ForgeConversationService<S> {
    conversation_repository: Arc<S>,
}

impl<S: ConversationRepository> ForgeConversationService<S> {
    /// Creates a new ForgeConversationService with the provided repository
    pub fn new(repo: Arc<S>) -> Self {
        Self { conversation_repository: repo }
    }
}

#[async_trait::async_trait]
impl<S: ConversationRepository> ConversationService for ForgeConversationService<S> {
    async fn modify_conversation<F, T>(&self, id: &ConversationId, f: F) -> Result<T>
    where
        F: FnOnce(&mut Conversation) -> T + Send,
        T: Send,
    {
        let mut conversation = self
            .conversation_repository
            .get_conversation(id)
            .await?
            .ok_or_else(|| forge_app::domain::Error::ConversationNotFound(*id))?;
        let out = f(&mut conversation);
        let _ = self
            .conversation_repository
            .upsert_conversation(conversation)
            .await?;
        Ok(out)
    }

    async fn find_conversation(&self, id: &ConversationId) -> Result<Option<Conversation>> {
        self.conversation_repository.get_conversation(id).await
    }

    async fn upsert_conversation(&self, conversation: Conversation) -> Result<()> {
        let _ = self
            .conversation_repository
            .upsert_conversation(conversation)
            .await?;
        Ok(())
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
        self.conversation_repository
            .delete_conversation(conversation_id)
            .await
    }

    async fn get_conversations_by_parent(
        &self,
        parent_id: &ConversationId,
    ) -> Result<Option<Vec<Conversation>>> {
        self.conversation_repository
            .get_conversations_by_parent(parent_id)
            .await
    }

    async fn get_parent_conversations(
        &self,
        limit: Option<usize>,
    ) -> Result<Option<Vec<Conversation>>> {
        self.conversation_repository
            .get_parent_conversations(limit)
            .await
    }

    async fn get_parent_conversations_lite(
        &self,
        limit: Option<usize>,
    ) -> Result<Option<Vec<ConversationSummary>>> {
        self.conversation_repository
            .get_parent_conversations_lite(limit)
            .await
    }

    async fn get_conversations_by_source(
        &self,
        source: &str,
        limit: Option<usize>,
    ) -> Result<Option<Vec<Conversation>>> {
        self.conversation_repository
            .get_conversations_by_source(source, limit)
            .await
    }

    async fn upsert_conversation_ref(&self, conversation: &Conversation) -> Result<()> {
        let _ = self
            .conversation_repository
            .upsert_conversation_ref(conversation)
            .await?;
        Ok(())
    }

    async fn search_conversations(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<Conversation>> {
        self.conversation_repository
            .search_conversations(query, limit)
            .await
    }

    async fn get_conversation_snippet(
        &self,
        conversation_id: &ConversationId,
        query: &str,
        token_count: usize,
    ) -> Result<Option<String>> {
        self.conversation_repository
            .get_conversation_snippet(conversation_id, query, token_count)
            .await
    }

    async fn optimize_fts_index(&self) -> Result<()> {
        let _ = self.conversation_repository.optimize_fts_index().await?;
        Ok(())
    }

    async fn update_parent_id(
        &self,
        conversation_id: &ConversationId,
        new_parent_id: Option<&ConversationId>,
    ) -> Result<()> {
        self.conversation_repository
            .update_parent_id(conversation_id, new_parent_id)
            .await
    }

    async fn get_conversations_by_cwd(
        &self,
        cwd: &str,
        limit: Option<usize>,
    ) -> Result<Option<Vec<Conversation>>> {
        self.conversation_repository
            .get_conversations_by_cwd(cwd, limit)
            .await
    }

    async fn rewind_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<Conversation>> {
        self.conversation_repository
            .rewind_conversation(conversation_id)
            .await
    }
}
