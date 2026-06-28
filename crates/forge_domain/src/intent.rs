//! ADR-103: Intent extraction and verification traits
//!
//! Traits for extracting semantic intent from conversations and storing
//! in the MemoryPort. Real implementations provided by thegent-memory v2;
//! this module provides stubs for the interface definition.

/// Scope for memory storage in the MemoryPort
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryScope {
    /// Raw session history and episodic memory (supermemory)
    Episodic,
    /// Agent persona and user context (letta subconscious)
    Identity,
    /// Code patterns and architecture decisions (cognee graph)
    ProjectKnowledge,
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Episodic => f.write_str("Episodic"),
            Self::Identity => f.write_str("Identity"),
            Self::ProjectKnowledge => f.write_str("ProjectKnowledge"),
        }
    }
}

/// Extracted intent snapshot for a conversation
#[derive(Debug, Clone)]
pub struct ExtractedIntent {
    /// Extracted episodic (session history) data
    pub episodic: serde_json::Value,
    /// Extracted identity (persona/context) data
    pub identity: serde_json::Value,
    /// Extracted project knowledge (patterns/architecture) data
    pub project_knowledge: serde_json::Value,
}

/// Result of intent extraction and MemoryPort storage
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    /// Conversation ID that was extracted
    pub conversation_id: String,
    /// UUID returned by MemoryPort.store() for episodic scope
    pub episodic_id: String,
    /// UUID returned by MemoryPort.store() for identity scope
    pub identity_id: String,
    /// UUID returned by MemoryPort.store() for project knowledge scope
    pub knowledge_id: String,
    /// SHA256 hash of the distilled intent snapshot
    pub intent_hash: String,
}

/// Trait for extracting intent from conversations
///
/// TODO (ADR-103): Real implementations provided by thegent-memory v2.
/// This is a documented interface; callers expect extract_intent and
/// verify_extraction signatures matching this trait.
#[async_trait::async_trait]
pub trait IntentExtractor: Send + Sync {
    /// Extract semantic intent from a conversation
    ///
    /// This produces three independent distilled blocks:
    /// - Episodic: raw session history
    /// - Identity: persona and human context
    /// - ProjectKnowledge: code patterns and architecture notes
    ///
    /// # Arguments
    /// * `conversation_id` - ID of conversation to extract from
    /// * `context` - Full conversation context blob
    ///
    /// # Errors
    /// Returns error if extraction fails (timeout, invalid format, etc.)
    ///
    /// # TODO
    /// Real implementation will be provided by thegent-memory v2 integration.
    async fn extract_intent(
        &self,
        conversation_id: &str,
        context: &str,
    ) -> anyhow::Result<ExtractedIntent>;

    /// Verify that extracted intent was successfully stored in MemoryPort
    ///
    /// Confirms that all three scopes (Episodic, Identity, ProjectKnowledge)
    /// are queryable in the MemoryPort and that the intent_hash matches
    /// the stored value.
    ///
    /// # Arguments
    /// * `conversation_id` - ID of conversation to verify
    /// * `intent_hash` - Expected SHA256 hash of the intent
    ///
    /// # Errors
    /// Returns error if verification fails (not found, hash mismatch, etc.)
    ///
    /// # TODO
    /// Real implementation will be provided by thegent-memory v2 integration.
    async fn verify_extraction(
        &self,
        conversation_id: &str,
        intent_hash: &str,
    ) -> anyhow::Result<bool>;
}

/// Noop implementation of IntentExtractor
///
/// Used as a placeholder when thegent-memory v2 is not available.
/// Both operations succeed with empty/identity results so callers
/// can run without a real memory integration wired in.
pub struct NoopIntentExtractor;

#[async_trait::async_trait]
impl IntentExtractor for NoopIntentExtractor {
    async fn extract_intent(
        &self,
        _conversation_id: &str,
        _context: &str,
    ) -> anyhow::Result<ExtractedIntent> {
        Ok(ExtractedIntent {
            episodic: serde_json::Value::Null,
            identity: serde_json::Value::Null,
            project_knowledge: serde_json::Value::Null,
        })
    }

    async fn verify_extraction(
        &self,
        _conversation_id: &str,
        _intent_hash: &str,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }
}
