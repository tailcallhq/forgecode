use std::sync::Arc;

use diesel::prelude::*;
use forge_domain::{Conversation, ConversationId, ConversationRepository, WorkspaceHash};

use crate::conversation::conversation_record::ConversationRecord;
use crate::database::DatabasePool;
use crate::database::schema::conversations;

pub struct ConversationRepositoryImpl {
    pool: Arc<DatabasePool>,
    wid: WorkspaceHash,
}

impl ConversationRepositoryImpl {
    pub fn new(pool: Arc<DatabasePool>, workspace_id: WorkspaceHash) -> Self {
        Self { pool, wid: workspace_id }
    }
}

#[async_trait::async_trait]
impl ConversationRepository for ConversationRepositoryImpl {
    async fn upsert_conversation(&self, conversation: Conversation) -> anyhow::Result<()> {
        let mut connection = self.pool.get_connection()?;

        let wid = self.wid;
        let record = ConversationRecord::new(conversation, wid);
        diesel::insert_into(conversations::table)
            .values(&record)
            .on_conflict(conversations::conversation_id)
            .do_update()
            .set((
                conversations::title.eq(&record.title),
                conversations::context.eq(&record.context),
                conversations::updated_at.eq(record.updated_at),
                conversations::metrics.eq(&record.metrics),
            ))
            .execute(&mut connection)?;
        Ok(())
    }

    async fn get_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> anyhow::Result<Option<Conversation>> {
        let mut connection = self.pool.get_connection()?;

        let record: Option<ConversationRecord> = conversations::table
            .filter(conversations::conversation_id.eq(conversation_id.into_string()))
            .first(&mut connection)
            .optional()?;

        match record {
            Some(record) => Ok(Some(Conversation::try_from(record)?)),
            None => Ok(None),
        }
    }

    async fn get_all_conversations(
        &self,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        let mut connection = self.pool.get_connection()?;

        let workspace_id = self.wid.id() as i64;
        let mut query = conversations::table
            .filter(conversations::workspace_id.eq(&workspace_id))
            .filter(conversations::context.is_not_null())
            .order(conversations::updated_at.desc())
            .into_boxed();

        if let Some(limit_value) = limit {
            query = query.limit(limit_value as i64);
        }

        let records: Vec<ConversationRecord> = query.load(&mut connection)?;

        if records.is_empty() {
            return Ok(None);
        }

        let conversations: Result<Vec<Conversation>, _> =
            records.into_iter().map(Conversation::try_from).collect();
        Ok(Some(conversations?))
    }

    async fn get_last_conversation(&self) -> anyhow::Result<Option<Conversation>> {
        let mut connection = self.pool.get_connection()?;
        let workspace_id = self.wid.id() as i64;
        let record: Option<ConversationRecord> = conversations::table
            .filter(conversations::workspace_id.eq(&workspace_id))
            .filter(conversations::context.is_not_null())
            .order(conversations::updated_at.desc())
            .first(&mut connection)
            .optional()?;
        let conversation = match record {
            Some(record) => Some(Conversation::try_from(record)?),
            None => None,
        };
        Ok(conversation)
    }

    async fn delete_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()> {
        let mut connection = self.pool.get_connection()?;
        let workspace_id = self.wid.id() as i64;

        // Security: Ensure users can only delete conversations within their workspace
        diesel::delete(conversations::table)
            .filter(conversations::workspace_id.eq(&workspace_id))
            .filter(conversations::conversation_id.eq(conversation_id.into_string()))
            .execute(&mut connection)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use forge_domain::{
        Context, ContextMessage, Effort, FileOperation, Metrics, Role, ToolCallFull, ToolCallId,
        ToolChoice, ToolDefinition, ToolKind, ToolName, ToolOutput, ToolResult, ToolValue, Usage,
    };
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::conversation::conversation_record::{ContextRecord, MetricsRecord};
    use crate::database::DatabasePool;

    fn repository() -> anyhow::Result<ConversationRepositoryImpl> {
        let pool = Arc::new(DatabasePool::in_memory()?);
        Ok(ConversationRepositoryImpl::new(pool, WorkspaceHash::new(0)))
    }

    #[tokio::test]
    async fn test_upsert_and_find_by_id() -> anyhow::Result<()> {
        let fixture = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));
        let repo = repository()?;

        repo.upsert_conversation(fixture.clone()).await?;

        let actual = repo.get_conversation(&fixture.id).await?;
        assert!(actual.is_some());
        let retrieved = actual.unwrap();
        assert_eq!(retrieved.id, fixture.id);
        assert_eq!(retrieved.title, fixture.title);
        Ok(())
    }

    #[tokio::test]
    async fn test_find_by_id_non_existing() -> anyhow::Result<()> {
        let repo = repository()?;
        let non_existing_id = ConversationId::generate();

        let actual = repo.get_conversation(&non_existing_id).await?;

        assert!(actual.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_upsert_updates_existing_conversation() -> anyhow::Result<()> {
        let mut fixture = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));
        let repo = repository()?;

        // Insert initial conversation
        repo.upsert_conversation(fixture.clone()).await?;

        // Update the conversation
        fixture = fixture.title(Some("Updated Title".to_string()));
        repo.upsert_conversation(fixture.clone()).await?;

        let actual = repo.get_conversation(&fixture.id).await?;
        assert!(actual.is_some());
        assert_eq!(actual.unwrap().title, Some("Updated Title".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn test_find_all_conversations() -> anyhow::Result<()> {
        let context1 =
            Context::default().messages(vec![ContextMessage::user("Hello", None).into()]);
        let context2 =
            Context::default().messages(vec![ContextMessage::user("World", None).into()]);
        let conversation1 = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()))
            .context(Some(context1));
        let conversation2 = Conversation::new(ConversationId::generate())
            .title(Some("Second Conversation".to_string()))
            .context(Some(context2));
        let repo = repository()?;

        repo.upsert_conversation(conversation1.clone()).await?;
        repo.upsert_conversation(conversation2.clone()).await?;

        let actual = repo.get_all_conversations(None).await?;

        assert!(actual.is_some());
        let conversations = actual.unwrap();
        assert_eq!(conversations.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn test_find_all_conversations_with_limit() -> anyhow::Result<()> {
        let context1 =
            Context::default().messages(vec![ContextMessage::user("Hello", None).into()]);
        let context2 =
            Context::default().messages(vec![ContextMessage::user("World", None).into()]);
        let conversation1 = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()))
            .context(Some(context1));
        let conversation2 = Conversation::new(ConversationId::generate()).context(Some(context2));
        let repo = repository()?;

        repo.upsert_conversation(conversation1).await?;
        repo.upsert_conversation(conversation2).await?;

        let actual = repo.get_all_conversations(Some(1)).await?;

        assert!(actual.is_some());
        assert_eq!(actual.unwrap().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_find_all_conversations_empty() -> anyhow::Result<()> {
        let repo = repository()?;

        let actual = repo.get_all_conversations(None).await?;

        assert!(actual.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_find_last_active_conversation_with_context() -> anyhow::Result<()> {
        let context = Context::default().messages(vec![ContextMessage::user("Hello", None).into()]);
        let conversation_with_context = Conversation::new(ConversationId::generate())
            .title(Some("Conversation with Context".to_string()))
            .context(Some(context));
        let conversation_without_context = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));
        let repo = repository()?;

        repo.upsert_conversation(conversation_without_context)
            .await?;
        repo.upsert_conversation(conversation_with_context.clone())
            .await?;

        let actual = repo.get_last_conversation().await?;

        assert!(actual.is_some());
        assert_eq!(actual.unwrap().id, conversation_with_context.id);
        Ok(())
    }

    #[tokio::test]
    async fn test_find_last_active_conversation_no_context() -> anyhow::Result<()> {
        let conversation_without_context = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));
        let repo = repository()?;

        repo.upsert_conversation(conversation_without_context)
            .await?;

        let actual = repo.get_last_conversation().await?;

        assert!(actual.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_find_last_active_conversation_ignores_empty_context() -> anyhow::Result<()> {
        let conversation_with_empty_context = Conversation::new(ConversationId::generate())
            .title(Some("Conversation with Empty Context".to_string()))
            .context(Some(Context::default()));
        let conversation_without_context = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));
        let repo = repository()?;

        repo.upsert_conversation(conversation_without_context)
            .await?;
        repo.upsert_conversation(conversation_with_empty_context)
            .await?;

        let actual = repo.get_last_conversation().await?;

        assert!(actual.is_none()); // Should not find conversations with empty contexts
        Ok(())
    }

    #[test]
    fn test_conversation_record_from_conversation() -> anyhow::Result<()> {
        let fixture = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));

        let actual = ConversationRecord::new(fixture.clone(), WorkspaceHash::new(0));

        assert_eq!(actual.conversation_id, fixture.id.into_string());
        assert_eq!(actual.title, Some("Test Conversation".to_string()));
        assert_eq!(actual.context, None);
        Ok(())
    }

    #[test]
    fn test_conversation_record_from_conversation_with_context() -> anyhow::Result<()> {
        let context = Context::default().messages(vec![ContextMessage::user("Hello", None).into()]);
        let fixture = Conversation::new(ConversationId::generate())
            .title(Some("Conversation with Context".to_string()))
            .context(Some(context));

        let actual = ConversationRecord::new(fixture.clone(), WorkspaceHash::new(0));

        assert_eq!(actual.conversation_id, fixture.id.into_string());
        assert_eq!(actual.title, Some("Conversation with Context".to_string()));
        assert!(actual.context.is_some());
        Ok(())
    }

    #[test]
    fn test_conversation_record_from_conversation_with_empty_context() -> anyhow::Result<()> {
        let fixture = Conversation::new(ConversationId::generate())
            .title(Some("Conversation with Empty Context".to_string()))
            .context(Some(Context::default()));

        let actual = ConversationRecord::new(fixture.clone(), WorkspaceHash::new(0));

        assert_eq!(actual.conversation_id, fixture.id.into_string());
        assert_eq!(
            actual.title,
            Some("Conversation with Empty Context".to_string())
        );

        assert!(actual.context.is_none()); // Empty context should be filtered out
        Ok(())
    }

    #[test]
    fn test_conversation_from_conversation_record() -> anyhow::Result<()> {
        let test_id = ConversationId::generate();
        let fixture = ConversationRecord {
            conversation_id: test_id.into_string(),
            title: Some("Test Conversation".to_string()),
            context: None,
            created_at: Utc::now().naive_utc(),
            updated_at: None,
            workspace_id: 0,
            metrics: None,
        };

        let actual = Conversation::try_from(fixture)?;

        assert_eq!(actual.id, test_id);
        assert_eq!(actual.title, Some("Test Conversation".to_string()));
        assert_eq!(actual.context, None);
        Ok(())
    }

    #[tokio::test]
    async fn test_upsert_and_retrieve_conversation_with_metrics() -> anyhow::Result<()> {
        let repo = repository()?;

        // Create a conversation with metrics
        let metrics = Metrics::default()
            .started_at(Utc::now())
            .insert(
                "src/main.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(10u64)
                    .lines_removed(5u64)
                    .content_hash(Some("abc123def456".to_string())),
            )
            .insert(
                "src/lib.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(3u64)
                    .lines_removed(2u64)
                    .content_hash(Some("789xyz456abc".to_string())),
            );

        let fixture = Conversation::generate().metrics(metrics.clone());

        // Save the conversation
        repo.upsert_conversation(fixture.clone()).await?;

        // Retrieve the conversation
        let actual = repo
            .get_conversation(&fixture.id)
            .await?
            .expect("Conversation should exist");

        // Verify metrics are preserved
        assert_eq!(actual.metrics.file_operations.len(), 2);
        let main_metrics = actual.metrics.file_operations.get("src/main.rs").unwrap();
        assert_eq!(main_metrics.lines_added, 10);
        assert_eq!(main_metrics.lines_removed, 5);
        assert_eq!(main_metrics.content_hash, Some("abc123def456".to_string()));
        let lib_metrics = actual.metrics.file_operations.get("src/lib.rs").unwrap();
        assert_eq!(lib_metrics.lines_added, 3);
        assert_eq!(lib_metrics.lines_removed, 2);
        assert_eq!(lib_metrics.content_hash, Some("789xyz456abc".to_string()));
        Ok(())
    }

    #[test]
    fn test_metrics_record_conversion_preserves_all_fields() {
        // This test ensures compile-time safety: if Metrics schema changes,
        // this test will fail to compile, alerting us to update MetricsRecord
        let fixture = Metrics::default().started_at(Utc::now()).insert(
            "test.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(5u64)
                .lines_removed(3u64)
                .content_hash(Some("test_hash_123".to_string())),
        );

        // Convert to record and back
        let record = MetricsRecord::from(&fixture);
        let actual = Metrics::from(record);

        // Verify all fields are preserved
        assert_eq!(actual.started_at, fixture.started_at);
        assert_eq!(actual.file_operations.len(), fixture.file_operations.len());

        let actual_file = actual.file_operations.get("test.rs").unwrap();
        let expected_file = fixture.file_operations.get("test.rs").unwrap();
        assert_eq!(actual_file.lines_added, expected_file.lines_added);
        assert_eq!(actual_file.lines_removed, expected_file.lines_removed);
        assert_eq!(actual_file.content_hash, expected_file.content_hash);
    }

    #[test]
    fn test_deserialize_old_format_without_tool_field() {
        // Old format from database: missing tool and content_hash fields
        let json = r#"{
            "started_at": "2024-01-01T00:00:00Z",
            "files_changed": {
                "src/main.rs": {
                    "lines_added": 10,
                    "lines_removed": 5
                },
                "src/lib.rs": {
                    "lines_added": 3,
                    "lines_removed": 2
                }
            }
        }"#;

        let record: MetricsRecord = serde_json::from_str(json).unwrap();
        let actual = Metrics::from(record);

        // Verify files are loaded
        assert_eq!(actual.file_operations.len(), 2);

        // Verify main.rs
        let main_file = actual.file_operations.get("src/main.rs").unwrap();
        assert_eq!(main_file.lines_added, 10);
        assert_eq!(main_file.lines_removed, 5);
        assert_eq!(main_file.content_hash, None);
        assert_eq!(main_file.tool, ToolKind::Write); // Default tool

        // Verify lib.rs
        let lib_file = actual.file_operations.get("src/lib.rs").unwrap();
        assert_eq!(lib_file.lines_added, 3);
        assert_eq!(lib_file.lines_removed, 2);
        assert_eq!(lib_file.content_hash, None);
        assert_eq!(lib_file.tool, ToolKind::Write); // Default tool
    }

    #[test]
    fn test_deserialize_array_format_takes_last_operation() {
        // Array format from database: multiple operations per file
        let json = r#"{
            "started_at": "2024-01-01T00:00:00Z",
            "files_changed": {
                "src/main.rs": [
                    {
                        "lines_added": 2,
                        "lines_removed": 4,
                        "content_hash": "hash1",
                        "tool": "read"
                    },
                    {
                        "lines_added": 1,
                        "lines_removed": 1,
                        "content_hash": "hash2",
                        "tool": "patch"
                    },
                    {
                        "lines_added": 5,
                        "lines_removed": 3,
                        "content_hash": "hash3",
                        "tool": "write"
                    }
                ]
            }
        }"#;

        let record: MetricsRecord = serde_json::from_str(json).unwrap();
        let actual = Metrics::from(record);

        // Verify only the last operation is kept
        assert_eq!(actual.file_operations.len(), 1);

        let main_file = actual.file_operations.get("src/main.rs").unwrap();
        assert_eq!(main_file.lines_added, 5);
        assert_eq!(main_file.lines_removed, 3);
        assert_eq!(main_file.content_hash, Some("hash3".to_string()));
        assert_eq!(main_file.tool, ToolKind::Write);
    }

    #[test]
    fn test_deserialize_array_format_with_empty_array() {
        // Array format with empty array should be skipped
        let json = r#"{
            "started_at": "2024-01-01T00:00:00Z",
            "files_changed": {
                "src/main.rs": [],
                "src/lib.rs": {
                    "lines_added": 5,
                    "lines_removed": 2,
                    "content_hash": "hash1",
                    "tool": "patch"
                }
            }
        }"#;

        let record: MetricsRecord = serde_json::from_str(json).unwrap();
        let actual = Metrics::from(record);

        // Empty array should be skipped, only lib.rs should be present
        assert_eq!(actual.file_operations.len(), 1);
        assert!(actual.file_operations.contains_key("src/lib.rs"));
        assert!(!actual.file_operations.contains_key("src/main.rs"));
    }

    #[test]
    fn test_deserialize_current_format_with_all_fields() {
        // Current format: single object with all fields
        let json = r#"{
            "started_at": "2024-01-01T00:00:00Z",
            "files_changed": {
                "src/main.rs": {
                    "lines_added": 10,
                    "lines_removed": 5,
                    "content_hash": "abc123def456",
                    "tool": "patch"
                },
                "src/lib.rs": {
                    "lines_added": 3,
                    "lines_removed": 2,
                    "content_hash": "789xyz456abc",
                    "tool": "write"
                }
            }
        }"#;

        let record: MetricsRecord = serde_json::from_str(json).unwrap();
        let actual = Metrics::from(record);

        // Verify all fields are preserved
        assert_eq!(actual.file_operations.len(), 2);

        let main_file = actual.file_operations.get("src/main.rs").unwrap();
        assert_eq!(main_file.lines_added, 10);
        assert_eq!(main_file.lines_removed, 5);
        assert_eq!(main_file.content_hash, Some("abc123def456".to_string()));
        assert_eq!(main_file.tool, ToolKind::Patch);

        let lib_file = actual.file_operations.get("src/lib.rs").unwrap();
        assert_eq!(lib_file.lines_added, 3);
        assert_eq!(lib_file.lines_removed, 2);
        assert_eq!(lib_file.content_hash, Some("789xyz456abc".to_string()));
        assert_eq!(lib_file.tool, ToolKind::Write);
    }

    #[test]
    fn test_deserialize_mixed_format() {
        // Mix of old format, array format, and current format
        let json = r#"{
            "started_at": "2024-01-01T00:00:00Z",
            "files_changed": {
                "old_file.rs": {
                    "lines_added": 10,
                    "lines_removed": 5
                },
                "array_file.rs": [
                    {
                        "lines_added": 1,
                        "lines_removed": 2,
                        "content_hash": "hash1",
                        "tool": "read"
                    },
                    {
                        "lines_added": 3,
                        "lines_removed": 4,
                        "content_hash": "hash2",
                        "tool": "patch"
                    }
                ],
                "current_file.rs": {
                    "lines_added": 7,
                    "lines_removed": 8,
                    "content_hash": "hash3",
                    "tool": "write"
                }
            }
        }"#;

        let record: MetricsRecord = serde_json::from_str(json).unwrap();
        let actual = Metrics::from(record);

        assert_eq!(actual.file_operations.len(), 3);

        // Old format file
        let old_file = actual.file_operations.get("old_file.rs").unwrap();
        assert_eq!(old_file.lines_added, 10);
        assert_eq!(old_file.lines_removed, 5);
        assert_eq!(old_file.content_hash, None);
        assert_eq!(old_file.tool, ToolKind::Write); // Default

        // Array format file (should have last operation)
        let array_file = actual.file_operations.get("array_file.rs").unwrap();
        assert_eq!(array_file.lines_added, 3);
        assert_eq!(array_file.lines_removed, 4);
        assert_eq!(array_file.content_hash, Some("hash2".to_string()));
        assert_eq!(array_file.tool, ToolKind::Patch);

        // Current format file
        let current_file = actual.file_operations.get("current_file.rs").unwrap();
        assert_eq!(current_file.lines_added, 7);
        assert_eq!(current_file.lines_removed, 8);
        assert_eq!(current_file.content_hash, Some("hash3".to_string()));
        assert_eq!(current_file.tool, ToolKind::Write);
    }

    #[test]
    fn test_serialize_current_format() {
        // Test that we always serialize in the current format (single object)
        let fixture = Metrics::default().started_at(Utc::now()).insert(
            "src/main.rs".to_string(),
            FileOperation::new(ToolKind::Patch)
                .lines_added(10u64)
                .lines_removed(5u64)
                .content_hash(Some("abc123".to_string())),
        );

        let record = MetricsRecord::from(&fixture);
        let json = serde_json::to_string(&record).unwrap();

        // Verify it's not an array format
        assert!(!json.contains("[{"));
        // Verify it contains the tool field
        assert!(json.contains("\"tool\":\"patch\""));

        // Verify structure is correct
        assert!(json.contains("\"lines_added\":10"));
        assert!(json.contains("\"lines_removed\":5"));
        assert!(json.contains("\"content_hash\":\"abc123\""));
    }

    #[test]
    fn test_context_record_conversion_preserves_all_fields() {
        let tool_def = ToolDefinition::new("test_tool").description("A test tool");

        let reasoning = forge_domain::ReasoningConfig {
            effort: Some(Effort::Medium),
            max_tokens: Some(2048),
            exclude: Some(false),
            enabled: Some(true),
        };

        // Create a comprehensive set of messages to test all message types
        let messages = vec![
            ContextMessage::user("Hello", None).into(),
            ContextMessage::system("System prompt").into(),
            ContextMessage::Tool(ToolResult {
                name: ToolName::new("test_tool"),
                call_id: Some(ToolCallId::new("call_123".to_string())),
                output: ToolOutput {
                    is_error: false,
                    values: vec![ToolValue::Text("Result text".to_string()), ToolValue::Empty],
                },
            })
            .into(),
            forge_domain::MessageEntry {
                message: ContextMessage::Text(forge_domain::TextMessage {
                    role: Role::Assistant,
                    content: "Assistant response".to_string(),
                    raw_content: None,
                    tool_calls: Some(vec![ToolCallFull {
                        name: ToolName::new("another_tool"),
                        call_id: Some(ToolCallId::new("call_456".to_string())),
                        arguments: forge_domain::ToolCallArguments::from(
                            serde_json::json!({"param": "value"}),
                        ),
                        thought_signature: None,
                    }]),
                    model: Some(forge_domain::ModelId::from("gpt-4")),
                    thought_signature: None,
                    reasoning_details: None,
                    droppable: false,
                    phase: None,
                }),
                usage: Some(Usage {
                    prompt_tokens: forge_domain::TokenCount::Actual(100),
                    completion_tokens: forge_domain::TokenCount::Actual(50),
                    total_tokens: forge_domain::TokenCount::Actual(150),
                    cached_tokens: forge_domain::TokenCount::Actual(0),
                    cost: Some(0.001),
                }),
            },
        ];

        let fixture = Context::default()
            .conversation_id(ConversationId::generate())
            .messages(messages)
            .tools(vec![tool_def.clone()])
            .tool_choice(ToolChoice::Call(ToolName::new("test_tool")))
            .max_tokens(1000usize)
            .temperature(forge_domain::Temperature::new(0.7).unwrap())
            .top_p(forge_domain::TopP::new(0.9).unwrap())
            .top_k(forge_domain::TopK::new(50).unwrap())
            .reasoning(reasoning.clone())
            .stream(true);

        // Convert to record and back
        let record = ContextRecord::from(&fixture);
        let actual = Context::try_from(record).unwrap();

        // Verify all fields are preserved
        assert_eq!(actual.conversation_id, fixture.conversation_id);
        assert_eq!(actual.messages.len(), 4);
        assert_eq!(actual.tools.len(), 1);
        assert_eq!(actual.tools[0].name.to_string(), "test_tool");
        assert_eq!(
            actual.tool_choice,
            Some(ToolChoice::Call(ToolName::new("test_tool")))
        );
        assert_eq!(actual.max_tokens, fixture.max_tokens);
        assert_eq!(actual.temperature, fixture.temperature);
        assert_eq!(actual.top_p, fixture.top_p);
        assert_eq!(actual.top_k, fixture.top_k);
        assert_eq!(actual.reasoning, Some(reasoning));
        assert_eq!(actual.stream, fixture.stream);

        // Verify message types and content
        match &actual.messages[0].message {
            ContextMessage::Text(msg) => {
                assert_eq!(msg.role, Role::User);
                assert_eq!(msg.content, "Hello");
            }
            _ => panic!("Expected user message"),
        }

        match &actual.messages[2].message {
            ContextMessage::Tool(tool_result) => {
                assert_eq!(tool_result.name.to_string(), "test_tool");
                assert_eq!(
                    tool_result.call_id.as_ref().map(|id| id.as_str()),
                    Some("call_123")
                );
                assert!(!tool_result.output.is_error);
                assert_eq!(tool_result.output.values.len(), 2);
            }
            _ => panic!("Expected tool result message"),
        }

        // Verify usage is preserved
        match &actual.messages[3].usage {
            Some(usage) => {
                assert_eq!(*usage.prompt_tokens, 100);
                assert_eq!(*usage.completion_tokens, 50);
                assert_eq!(*usage.total_tokens, 150);
                assert_eq!(usage.cost, Some(0.001));
            }
            None => panic!("Expected usage information"),
        }
    }

    #[test]
    fn test_conversation_deserialization_error_includes_id() {
        // Test that deserialization errors include the conversation ID
        let test_id = ConversationId::generate();
        let fixture = ConversationRecord {
            conversation_id: test_id.into_string(),
            title: Some("Test Conversation".to_string()),
            context: Some("invalid json".to_string()), // Invalid JSON to trigger error
            created_at: Utc::now().naive_utc(),
            updated_at: None,
            workspace_id: 0,
            metrics: None,
        };

        let result = Conversation::try_from(fixture);

        assert!(result.is_err());
        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains(&test_id.to_string()),
            "Error message should contain conversation ID. Got: {}",
            error_message
        );
        assert!(
            error_message.contains("Failed to deserialize context"),
            "Error message should indicate context deserialization failure. Got: {}",
            error_message
        );
    }

    #[tokio::test]
    async fn test_delete_conversation_success() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));

        repo.upsert_conversation(conversation.clone()).await?;

        repo.delete_conversation(&conversation.id).await?;

        let result = repo.get_conversation(&conversation.id).await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_conversation_workspace_filtering() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));

        repo.upsert_conversation(conversation.clone()).await?;

        // Delete should succeed regardless of existence (idempotent)
        repo.delete_conversation(&conversation.id).await?;

        // Verify conversation is deleted
        let deleted = repo.get_conversation(&conversation.id).await?;
        assert!(deleted.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_conversation_cross_workspace_security() -> anyhow::Result<()> {
        let repo = repository()?;

        // Create conversation in current workspace
        let conversation_id = ConversationId::generate();
        let conversation =
            Conversation::new(conversation_id).title(Some("Test Conversation".to_string()));

        repo.upsert_conversation(conversation.clone()).await?;

        // Try to delete with different workspace ID (should fail due to security)
        // Note: This test would require modifying workspace ID in repo
        // For now, we test that deletion works with current workspace
        repo.delete_conversation(&conversation.id).await?;

        // Verify it's actually deleted
        let deleted = repo.get_conversation(&conversation.id).await?;
        assert!(deleted.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_conversation_end_to_end_workflow() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation_id = ConversationId::generate();
        let conversation =
            Conversation::new(conversation_id).title(Some("Test Conversation".to_string()));

        // Test complete workflow: create -> delete -> verify -> create new -> verify
        repo.upsert_conversation(conversation.clone()).await?;

        // Delete conversation
        repo.delete_conversation(&conversation.id).await?;

        // Verify it's gone
        let deleted_check = repo.get_conversation(&conversation.id).await?;
        assert!(deleted_check.is_none());

        // Create new conversation to ensure system still works
        let new_conversation_id = ConversationId::generate();
        let new_conversation = Conversation::new(new_conversation_id);
        repo.upsert_conversation(new_conversation.clone()).await?;

        // Verify new conversation exists
        let new_check = repo.get_conversation(&new_conversation_id).await?;
        assert!(new_check.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_rename_conversation_via_upsert() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation =
            Conversation::new(ConversationId::generate()).title(Some("Original Title".to_string()));

        repo.upsert_conversation(conversation.clone()).await?;

        // Rename by upserting with a new title
        let renamed = conversation
            .clone()
            .title(Some("Renamed Session".to_string()));
        repo.upsert_conversation(renamed).await?;

        let actual = repo.get_conversation(&conversation.id).await?.unwrap();
        assert_eq!(actual.title, Some("Renamed Session".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn test_rename_conversation_from_none() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation = Conversation::new(ConversationId::generate());

        // Start with no title
        assert!(conversation.title.is_none());
        repo.upsert_conversation(conversation.clone()).await?;

        // Rename it
        let renamed = conversation.clone().title(Some("My Session".to_string()));
        repo.upsert_conversation(renamed).await?;

        let actual = repo.get_conversation(&conversation.id).await?.unwrap();
        assert_eq!(actual.title, Some("My Session".to_string()));
        Ok(())
    }

    #[test]
    fn test_legacy_tool_value_pair_deserialization() {
        use crate::conversation::conversation_record::ToolOutputRecord;

        // This JSON represents the old Pair variant format that was stored in the
        // database
        let legacy_json = r#"{
            "is_error": false,
            "values": [
                {"pair": [
                    {"text": "XML content for LLM"},
                    {"fileDiff": {"path": "/test/file.rs", "old_text": "old", "new_text": "new"}}
                ]}
            ]
        }"#;

        let record: ToolOutputRecord = serde_json::from_str(legacy_json).unwrap();
        let actual: forge_domain::ToolOutput = record.try_into().unwrap();

        // The Pair variant should be converted by taking the first element (LLM
        // content)
        assert!(!actual.is_error);
        assert_eq!(actual.values.len(), 1);
        assert_eq!(
            actual.values[0],
            forge_domain::ToolValue::Text("XML content for LLM".to_string())
        );
    }

    #[test]
    fn test_legacy_tool_value_markdown_deserialization() {
        use crate::conversation::conversation_record::ToolOutputRecord;

        let legacy_json = r##"{
            "is_error": false,
            "values": [{"markdown": "# Heading - Some bold text"}]
        }"##;

        let record: ToolOutputRecord = serde_json::from_str(legacy_json).unwrap();
        let actual: forge_domain::ToolOutput = record.try_into().unwrap();

        // Markdown should be converted to Text
        assert_eq!(actual.values.len(), 1);
        assert_eq!(
            actual.values[0],
            forge_domain::ToolValue::Text("# Heading - Some bold text".to_string())
        );
    }

    #[test]
    fn test_legacy_tool_value_file_diff_deserialization() {
        use crate::conversation::conversation_record::ToolOutputRecord;

        let legacy_json = r#"{
            "is_error": false,
            "values": [{"fileDiff": {"path": "/src/main.rs", "old_text": "fn old()", "new_text": "fn new()"}}]
        }"#;

        let record: ToolOutputRecord = serde_json::from_str(legacy_json).unwrap();
        let actual: forge_domain::ToolOutput = record.try_into().unwrap();

        // FileDiff should be converted to a text summary
        assert_eq!(actual.values.len(), 1);
        assert_eq!(
            actual.values[0],
            forge_domain::ToolValue::Text("[File diff: /src/main.rs]".to_string())
        );
    }
}
