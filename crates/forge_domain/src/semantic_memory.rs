#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use pretty_assertions::assert_eq;

    use crate::{ConversationId, MemoryScope, WorkspaceId};

    use super::*;

    fn fixture_namespace() -> SemanticMemoryNamespace {
        SemanticMemoryNamespace::new(WorkspaceId::generate())
    }

    fn fixture_provenance(namespace: SemanticMemoryNamespace) -> SemanticMemoryProvenance {
        SemanticMemoryProvenance::new(
            ConversationId::generate(),
            namespace,
            "conversation:fixture",
        )
    }

    struct RecordingPort {
        forgotten: Mutex<Vec<SemanticMemoryIdentity>>,
    }

    #[async_trait::async_trait]
    impl SemanticMemoryPort for RecordingPort {
        async fn store(
            &self,
            _write: SemanticMemoryWrite,
        ) -> Result<SemanticMemoryIdentity, SemanticMemoryError> {
            Err(SemanticMemoryError::InvalidResponse(
                "store is not used by this fixture".to_string(),
            ))
        }

        async fn recall(
            &self,
            _query: SemanticMemoryQuery,
            _budget: SemanticMemoryBudget,
        ) -> Result<Vec<SemanticMemoryRecord>, SemanticMemoryError> {
            Ok(Vec::new())
        }

        async fn forget(
            &self,
            identity: SemanticMemoryIdentity,
        ) -> Result<(), SemanticMemoryError> {
            self.forgotten.lock().unwrap().push(identity);
            Ok(())
        }

        fn provider_name(&self) -> &'static str {
            "recording"
        }
    }

    #[test]
    fn episodic_scope_is_accepted_and_other_existing_scopes_are_rejected() {
        let actual = SemanticMemoryScope::try_from(MemoryScope::Episodic);
        let expected = Ok(SemanticMemoryScope::Episodic);
        assert_eq!(actual, expected);

        let actual = SemanticMemoryScope::try_from(MemoryScope::Identity);
        assert_eq!(
            actual,
            Err(SemanticMemoryError::UnsupportedScope(MemoryScope::Identity))
        );

        let actual = SemanticMemoryScope::try_from(MemoryScope::ProjectKnowledge);
        assert_eq!(
            actual,
            Err(SemanticMemoryError::UnsupportedScope(
                MemoryScope::ProjectKnowledge
            ))
        );
    }

    #[test]
    fn query_rejects_empty_text_excessive_limit_and_non_finite_min_score() {
        let namespace = fixture_namespace();
        let actual = SemanticMemoryQuery::new(namespace.clone(), "", 1, None);
        assert_eq!(actual, Err(SemanticMemoryError::EmptyQuery));

        let actual = SemanticMemoryQuery::new(
            namespace.clone(),
            "find the rollback plan",
            SemanticMemoryQuery::MAX_LIMIT + 1,
            None,
        );
        assert_eq!(
            actual,
            Err(SemanticMemoryError::QueryLimitExceeded {
                requested: SemanticMemoryQuery::MAX_LIMIT + 1,
                maximum: SemanticMemoryQuery::MAX_LIMIT,
            })
        );

        let actual =
            SemanticMemoryQuery::new(namespace, "find the rollback plan", 1, Some(f32::NAN));
        assert_eq!(actual, Err(SemanticMemoryError::InvalidMinScore));
    }

    #[test]
    fn recall_budget_rejects_zero_and_excessive_bytes() {
        let actual = SemanticMemoryBudget::new(0);
        assert_eq!(actual, Err(SemanticMemoryError::InvalidBudget));

        let actual = SemanticMemoryBudget::new(SemanticMemoryBudget::MAX_BYTES + 1);
        assert_eq!(
            actual,
            Err(SemanticMemoryError::BudgetExceeded {
                requested: SemanticMemoryBudget::MAX_BYTES + 1,
                maximum: SemanticMemoryBudget::MAX_BYTES,
            })
        );
    }

    #[test]
    fn records_sort_by_score_then_key_and_keep_provenance() {
        let namespace = fixture_namespace();
        let provenance = fixture_provenance(namespace.clone());
        let low = SemanticMemoryRecord::try_new(
            SemanticMemoryIdentity::new(SemanticMemoryScope::Episodic, namespace.clone(), "zeta")
                .unwrap(),
            "low result",
            0.5,
            provenance.clone(),
        )
        .unwrap();
        let alpha = SemanticMemoryRecord::try_new(
            SemanticMemoryIdentity::new(SemanticMemoryScope::Episodic, namespace.clone(), "alpha")
                .unwrap(),
            "first tied result",
            0.9,
            provenance.clone(),
        )
        .unwrap();
        let beta = SemanticMemoryRecord::try_new(
            SemanticMemoryIdentity::new(SemanticMemoryScope::Episodic, namespace, "beta").unwrap(),
            "second tied result",
            0.9,
            provenance.clone(),
        )
        .unwrap();

        let actual = SemanticMemoryRecord::ranked(vec![low, beta, alpha]);
        let actual_keys: Vec<&str> = actual.iter().map(|record| record.identity().id()).collect();
        let expected_keys = vec!["alpha", "beta", "zeta"];
        assert_eq!(actual_keys, expected_keys);
        assert_eq!(actual[0].provenance(), &provenance);
    }

    #[test]
    fn query_and_record_identity_keep_workspace_namespaces_isolated() {
        let first_namespace = fixture_namespace();
        let second_namespace = fixture_namespace();
        let query =
            SemanticMemoryQuery::new(first_namespace.clone(), "find rollback", 1, None).unwrap();
        assert_eq!(query.namespace(), &first_namespace);

        let identity =
            SemanticMemoryIdentity::new(SemanticMemoryScope::Episodic, first_namespace, "memory-1")
                .unwrap();
        let actual = SemanticMemoryRecord::try_new(
            identity,
            "cross-workspace record",
            0.9,
            fixture_provenance(second_namespace),
        );
        assert_eq!(actual, Err(SemanticMemoryError::NamespaceMismatch));
    }

    #[tokio::test]
    async fn forget_receives_complete_scoped_identity_and_is_idempotent_at_the_port_boundary() {
        let namespace = fixture_namespace();
        let identity = SemanticMemoryIdentity::new(
            SemanticMemoryScope::Episodic,
            namespace,
            "provider-memory-id",
        )
        .unwrap();
        let fixture = RecordingPort { forgotten: Mutex::new(Vec::new()) };

        fixture.forget(identity.clone()).await.unwrap();
        fixture.forget(identity.clone()).await.unwrap();

        let actual = fixture.forgotten.lock().unwrap().clone();
        let expected = vec![identity.clone(), identity];
        assert_eq!(actual, expected);
    }

    #[test]
    fn error_taxonomy_marks_only_unavailable_as_fallback_eligible() {
        let unavailable = SemanticMemoryError::Unavailable("sidecar unavailable".to_string());
        assert!(unavailable.allows_fts_fallback());

        for error in [
            SemanticMemoryError::Backend { status: 401, body: "unauthorized".to_string() },
            SemanticMemoryError::InvalidResponse("non-finite vector".to_string()),
            SemanticMemoryError::InvalidMinScore,
        ] {
            assert!(!error.allows_fts_fallback());
        }
    }
}
use async_trait::async_trait;
use thiserror::Error;

use crate::{ConversationId, MemoryScope, WorkspaceId};

/// The only memory scope implemented by the first semantic-memory slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticMemoryScope {
    /// Conversation history recalled on demand.
    Episodic,
}

impl TryFrom<MemoryScope> for SemanticMemoryScope {
    type Error = SemanticMemoryError;

    fn try_from(scope: MemoryScope) -> Result<Self, Self::Error> {
        match scope {
            MemoryScope::Episodic => Ok(Self::Episodic),
            scope => Err(SemanticMemoryError::UnsupportedScope(scope)),
        }
    }
}

/// The tenant boundary that isolates semantic memory to one workspace.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemanticMemoryNamespace {
    workspace_id: WorkspaceId,
}

impl SemanticMemoryNamespace {
    /// Creates the semantic-memory namespace for one workspace.
    pub fn new(workspace_id: WorkspaceId) -> Self {
        Self { workspace_id }
    }

    /// Returns the workspace that owns this namespace.
    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
}

/// A provider-stable identity for a semantic-memory record.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemanticMemoryIdentity {
    scope: SemanticMemoryScope,
    namespace: SemanticMemoryNamespace,
    id: String,
}

impl SemanticMemoryIdentity {
    /// Creates a scoped provider identity.
    ///
    /// # Errors
    /// Returns an error when the provider identifier is blank.
    pub fn new(
        scope: SemanticMemoryScope,
        namespace: SemanticMemoryNamespace,
        id: impl Into<String>,
    ) -> Result<Self, SemanticMemoryError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(SemanticMemoryError::EmptyMemoryId);
        }
        Ok(Self { scope, namespace, id })
    }

    /// Returns the record scope.
    pub fn scope(&self) -> SemanticMemoryScope {
        self.scope
    }

    /// Returns the namespace that owns this record.
    pub fn namespace(&self) -> &SemanticMemoryNamespace {
        &self.namespace
    }

    /// Returns the provider-assigned identifier.
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// The provenance retained with every recalled memory record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticMemoryProvenance {
    conversation_id: ConversationId,
    namespace: SemanticMemoryNamespace,
    source_key: String,
}

impl SemanticMemoryProvenance {
    /// Creates provenance for a record derived from one conversation.
    pub fn new(
        conversation_id: ConversationId,
        namespace: SemanticMemoryNamespace,
        source_key: impl Into<String>,
    ) -> Self {
        Self { conversation_id, namespace, source_key: source_key.into() }
    }

    /// Returns the conversation from which this record was derived.
    pub fn conversation_id(&self) -> ConversationId {
        self.conversation_id
    }

    /// Returns the workspace that owns this record.
    pub fn workspace_id(&self) -> &WorkspaceId {
        self.namespace.workspace_id()
    }

    /// Returns the namespace that owns the source conversation.
    pub fn namespace(&self) -> &SemanticMemoryNamespace {
        &self.namespace
    }

    /// Returns the opaque caller-assigned source key.
    pub fn source_key(&self) -> &str {
        &self.source_key
    }
}

/// A validated natural-language semantic-memory recall query.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticMemoryQuery {
    namespace: SemanticMemoryNamespace,
    text: String,
    limit: usize,
    min_score: Option<f32>,
}

impl SemanticMemoryQuery {
    /// Default number of records requested when callers do not need a wider recall.
    pub const DEFAULT_LIMIT: usize = 10;
    /// Hard upper bound that keeps a recall request bounded before any adapter runs.
    pub const MAX_LIMIT: usize = 100;

    /// Creates a validated recall query.
    ///
    /// # Errors
    /// Returns an error for blank text, an out-of-range limit, or a non-finite score.
    pub fn new(
        namespace: SemanticMemoryNamespace,
        text: impl Into<String>,
        limit: usize,
        min_score: Option<f32>,
    ) -> Result<Self, SemanticMemoryError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(SemanticMemoryError::EmptyQuery);
        }
        if limit == 0 || limit > Self::MAX_LIMIT {
            return Err(SemanticMemoryError::QueryLimitExceeded {
                requested: limit,
                maximum: Self::MAX_LIMIT,
            });
        }
        if min_score.is_some_and(|score| !score.is_finite()) {
            return Err(SemanticMemoryError::InvalidMinScore);
        }
        Ok(Self { namespace, text, limit, min_score })
    }

    /// Returns the namespace searched by this query.
    pub fn namespace(&self) -> &SemanticMemoryNamespace {
        &self.namespace
    }

    /// Returns the query text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the maximum number of records to recall.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Returns the optional lower relevance threshold.
    pub fn min_score(&self) -> Option<f32> {
        self.min_score
    }
}

/// A hard cap on recalled content before it can enter any prompt context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticMemoryBudget(usize);

impl SemanticMemoryBudget {
    /// Default prompt-content budget for semantic recall.
    pub const DEFAULT_BYTES: usize = 8 * 1024;
    /// Largest permitted prompt-content budget for a single recall operation.
    pub const MAX_BYTES: usize = 64 * 1024;

    /// Creates a validated recall-content budget.
    ///
    /// # Errors
    /// Returns an error for zero or excessive byte budgets.
    pub fn new(bytes: usize) -> Result<Self, SemanticMemoryError> {
        if bytes == 0 {
            return Err(SemanticMemoryError::InvalidBudget);
        }
        if bytes > Self::MAX_BYTES {
            return Err(SemanticMemoryError::BudgetExceeded {
                requested: bytes,
                maximum: Self::MAX_BYTES,
            });
        }
        Ok(Self(bytes))
    }

    /// Returns the maximum recalled-content size in bytes.
    pub fn bytes(self) -> usize {
        self.0
    }
}

/// A single result returned from a semantic-memory recall operation.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticMemoryRecord {
    identity: SemanticMemoryIdentity,
    content: String,
    score: f32,
    provenance: SemanticMemoryProvenance,
}

impl SemanticMemoryRecord {
    /// Creates a record with a finite relevance score.
    ///
    /// # Errors
    /// Returns an error when the relevance score is non-finite.
    pub fn try_new(
        identity: SemanticMemoryIdentity,
        content: impl Into<String>,
        score: f32,
        provenance: SemanticMemoryProvenance,
    ) -> Result<Self, SemanticMemoryError> {
        if !score.is_finite() {
            return Err(SemanticMemoryError::InvalidResponse(
                "semantic-memory score must be finite".to_string(),
            ));
        }
        if identity.namespace != provenance.namespace {
            return Err(SemanticMemoryError::NamespaceMismatch);
        }
        Ok(Self { identity, content: content.into(), score, provenance })
    }

    /// Returns records ordered by descending score and ascending key for stable ties.
    pub fn ranked(mut records: Vec<Self>) -> Vec<Self> {
        records.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.identity.id.cmp(&right.identity.id))
                .then_with(|| left.provenance.source_key.cmp(&right.provenance.source_key))
        });
        records
    }

    /// Returns the record scope.
    pub fn scope(&self) -> SemanticMemoryScope {
        self.identity.scope
    }

    /// Returns the scoped provider identity for this record.
    pub fn identity(&self) -> &SemanticMemoryIdentity {
        &self.identity
    }

    /// Returns the recalled text payload.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Returns the adapter-provided relevance score.
    pub fn score(&self) -> f32 {
        self.score
    }

    /// Returns the record provenance.
    pub fn provenance(&self) -> &SemanticMemoryProvenance {
        &self.provenance
    }
}

/// A write command that contains source content but no recall-only score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticMemoryWrite {
    scope: SemanticMemoryScope,
    namespace: SemanticMemoryNamespace,
    content: String,
    provenance: SemanticMemoryProvenance,
}

impl SemanticMemoryWrite {
    /// Creates a scoped write command for one source conversation.
    ///
    /// # Errors
    /// Returns an error when the write namespace does not match its provenance.
    pub fn new(
        scope: SemanticMemoryScope,
        namespace: SemanticMemoryNamespace,
        content: impl Into<String>,
        provenance: SemanticMemoryProvenance,
    ) -> Result<Self, SemanticMemoryError> {
        if namespace != provenance.namespace {
            return Err(SemanticMemoryError::NamespaceMismatch);
        }
        Ok(Self { scope, namespace, content: content.into(), provenance })
    }

    /// Returns the write scope.
    pub fn scope(&self) -> SemanticMemoryScope {
        self.scope
    }

    /// Returns the namespace that owns the write.
    pub fn namespace(&self) -> &SemanticMemoryNamespace {
        &self.namespace
    }

    /// Returns the content to store.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Returns the source provenance.
    pub fn provenance(&self) -> &SemanticMemoryProvenance {
        &self.provenance
    }
}

/// A provider-agnostic semantic-memory operation boundary.
#[async_trait]
pub trait SemanticMemoryPort: Send + Sync {
    /// Stores one Episodic memory and returns its provider-assigned identifier.
    async fn store(
        &self,
        write: SemanticMemoryWrite,
    ) -> Result<SemanticMemoryIdentity, SemanticMemoryError>;

    /// Recalls records matching a validated query.
    async fn recall(
        &self,
        query: SemanticMemoryQuery,
        budget: SemanticMemoryBudget,
    ) -> Result<Vec<SemanticMemoryRecord>, SemanticMemoryError>;

    /// Forgets a scoped memory identity. Missing identities must be treated as success.
    async fn forget(&self, identity: SemanticMemoryIdentity) -> Result<(), SemanticMemoryError>;

    /// Returns a stable label for diagnostics without exposing credentials.
    fn provider_name(&self) -> &'static str;
}

/// Errors produced at the provider-agnostic semantic-memory boundary.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SemanticMemoryError {
    /// The requested Forge memory scope is outside the F3 Episodic slice.
    #[error("unsupported semantic-memory scope: {0}")]
    UnsupportedScope(MemoryScope),
    /// A recall query has no meaningful text.
    #[error("semantic-memory query text must not be blank")]
    EmptyQuery,
    /// A recall query requested an unsupported number of results.
    #[error("semantic-memory query limit {requested} exceeds maximum {maximum}")]
    QueryLimitExceeded { requested: usize, maximum: usize },
    /// A provider identity is blank and cannot be addressed safely.
    #[error("semantic-memory provider identifier must not be blank")]
    EmptyMemoryId,
    /// A record or write combines provenance from a different namespace.
    #[error("semantic-memory identity and provenance namespaces must match")]
    NamespaceMismatch,
    /// A score is not finite and cannot be used safely for ranking.
    #[error("semantic-memory minimum score must be finite")]
    InvalidMinScore,
    /// A recall budget is zero bytes.
    #[error("semantic-memory budget must be greater than zero")]
    InvalidBudget,
    /// A recall budget exceeds the domain hard cap.
    #[error("semantic-memory budget {requested} exceeds maximum {maximum}")]
    BudgetExceeded { requested: usize, maximum: usize },
    /// The provider is temporarily unavailable and may use the FTS fallback.
    #[error("semantic-memory provider unavailable: {0}")]
    Unavailable(String),
    /// The provider returned a non-success status and body.
    #[error("semantic-memory backend returned status {status}: {body}")]
    Backend { status: u16, body: String },
    /// The provider response cannot safely be converted into a domain record.
    #[error("invalid semantic-memory provider response: {0}")]
    InvalidResponse(String),
}

impl SemanticMemoryError {
    /// Returns whether this error is eligible for a semantic-to-FTS fallback.
    pub fn allows_fts_fallback(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}
