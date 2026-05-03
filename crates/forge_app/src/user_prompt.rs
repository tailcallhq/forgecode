use std::ops::Deref;
use std::sync::Arc;

use forge_domain::{Agent, *};
use serde_json::json;
use tracing::debug;

use crate::{AttachmentService, EnvironmentInfra, TemplateEngine, TerminalContextService};

/// Service responsible for setting user prompts in the conversation context
#[derive(Clone)]
pub struct UserPromptGenerator<S> {
    services: Arc<S>,
    agent: Agent,
    event: Event,
    current_time: chrono::DateTime<chrono::Local>,
}

impl<S: AttachmentService + EnvironmentInfra<Config = forge_config::ForgeConfig>>
    UserPromptGenerator<S>
{
    /// Creates a new UserPromptService
    pub fn new(
        service: Arc<S>,
        agent: Agent,
        event: Event,
        current_time: chrono::DateTime<chrono::Local>,
    ) -> Self {
        Self { services: service, agent, event, current_time }
    }

    /// Builds the pending-turn messages for this user input. The
    /// conversation's `context` (canonical) is left untouched; halted
    /// turns drop the pending without ever persisting to canonical.
    pub async fn generate(
        &self,
        conversation: Conversation,
    ) -> anyhow::Result<(Conversation, PendingTurn)> {
        let is_resume = conversation
            .context
            .as_ref()
            .map(|ctx| ctx.messages.iter().any(|msg| msg.has_role(Role::User)))
            .unwrap_or(false);

        let mut pending = PendingTurn::default();

        let content = self
            .build_rendered_message(&conversation, &mut pending)
            .await?;

        if is_resume {
            self.build_todos_on_resume(&conversation, &mut pending);
        }

        self.build_additional_context(&mut pending);

        let conversation = if let Some(content) = content {
            self.build_attachments(conversation, &mut pending, &content)
                .await?
        } else {
            conversation
        };

        Ok((conversation, pending))
    }

    /// Pushes the todo-resume reminder (if any) into pending. Reads todos
    /// from session metrics; droppable so later compaction can drop it.
    fn build_todos_on_resume(&self, conversation: &Conversation, pending: &mut PendingTurn) {
        let todos = &conversation.metrics.todos;
        if todos.is_empty() {
            return;
        }

        let todo_content = self.format_todos_as_markdown(todos);
        let todo_message = TextMessage {
            role: Role::User,
            content: todo_content,
            raw_content: None,
            tool_calls: None,
            thought_signature: None,
            reasoning_details: None,
            model: Some(self.agent.model.clone()),
            droppable: true,
            phase: None,
        };
        pending.push_user_input(ContextMessage::Text(todo_message));
    }

    /// Formats todos as a markdown checklist
    fn format_todos_as_markdown(&self, todos: &[Todo]) -> String {
        use std::fmt::Write;

        let mut content = String::from("**Current task list:**\n\n");

        for todo in todos {
            let checkbox = match todo.status {
                TodoStatus::Completed => "[DONE]",
                TodoStatus::InProgress => "[IN_PROGRESS]",
                TodoStatus::Pending => "[PENDING]",
                TodoStatus::Cancelled => "[CANCELLED]",
            };

            writeln!(content, "- {} {}", checkbox, todo.content)
                .expect("Writing to String should not fail");
        }

        content
    }

    /// Pushes the piped additional-context message (if any) into pending.
    /// Droppable so later compaction can drop it.
    fn build_additional_context(&self, pending: &mut PendingTurn) {
        let Some(piped_input) = &self.event.additional_context else {
            return;
        };
        let piped_message = TextMessage {
            role: Role::User,
            content: piped_input.clone(),
            raw_content: None,
            tool_calls: None,
            thought_signature: None,
            reasoning_details: None,
            model: Some(self.agent.model.clone()),
            droppable: true,
            phase: None,
        };
        pending.push_user_input(ContextMessage::Text(piped_message));
    }

    /// Renders the user's primary message into pending and returns the
    /// rendered content so attachment parsing can scan it.
    async fn build_rendered_message(
        &self,
        conversation: &Conversation,
        pending: &mut PendingTurn,
    ) -> anyhow::Result<Option<String>> {
        let event_value = self.event.value.clone();
        let template_engine = TemplateEngine::default();

        // Treat it as feedback when canonical already has a user message.
        let has_user_messages = conversation
            .context
            .as_ref()
            .map(|ctx| ctx.messages.iter().any(|msg| msg.has_role(Role::User)))
            .unwrap_or(false);

        let content = if let Some(user_prompt) = &self.agent.user_prompt
            && self.event.value.is_some()
        {
            let user_input = self
                .event
                .value
                .as_ref()
                .and_then(|v| v.as_user_prompt().map(|u| u.as_str().to_string()))
                .unwrap_or_default();
            let mut event_context = EventContext::new(EventContextValue::new(user_input))
                .current_date(self.current_time.format("%Y-%m-%d").to_string());

            if has_user_messages {
                event_context = event_context.into_feedback();
            } else {
                event_context = event_context.into_task();
            }

            debug!(event_context = ?event_context, "Event context");

            let event_context = match self.event.value.as_ref().and_then(|v| v.as_command()) {
                Some(command) => {
                    let rendered_prompt = template_engine.render_template(
                        command.template.clone(),
                        &json!({"parameters": command.parameters.join(" ")}),
                    )?;
                    event_context.event(EventContextValue::new(rendered_prompt))
                }
                None => event_context,
            };

            let event_context =
                match TerminalContextService::new(self.services.clone()).get_terminal_context() {
                    Some(ctx) => event_context.terminal_context(Some(ctx)),
                    None => event_context,
                };

            Some(
                template_engine.render_template(
                    Template::new(user_prompt.template.as_str()),
                    &event_context,
                )?,
            )
        } else {
            event_value
                .as_ref()
                .and_then(|v| v.as_user_prompt().map(|p| p.deref().to_owned()))
        };

        if let Some(content) = &content {
            let message = TextMessage {
                role: Role::User,
                content: content.clone(),
                raw_content: event_value,
                tool_calls: None,
                thought_signature: None,
                reasoning_details: None,
                model: Some(self.agent.model.clone()),
                droppable: false,
                phase: None,
            };
            pending.push_user_input(ContextMessage::Text(message));
        }

        Ok(content)
    }

    /// Parses attachments out of the rendered content and routes them into
    /// pending. Metrics (which are session-wide, not canonical) still
    /// update on `conversation` so read-operation tracking is preserved
    /// regardless of turn outcome.
    async fn build_attachments(
        &self,
        mut conversation: Conversation,
        pending: &mut PendingTurn,
        content: &str,
    ) -> anyhow::Result<Conversation> {
        let attachments = self.services.attachments(content).await?;

        let mut metrics = conversation.metrics.clone();
        for attachment in &attachments {
            // Use the raw content_hash (pre-line-numbering) so the external-
            // change detector's file-on-disk hash matches and doesn't raise
            // a spurious "modified externally" warning on the next turn.
            if let AttachmentContent::FileContent { info, .. } = &attachment.content {
                metrics = metrics.insert(
                    attachment.path.clone(),
                    FileOperation::new(ToolKind::Read)
                        .content_hash(Some(info.content_hash.clone())),
                );
            }
        }
        conversation.metrics = metrics;

        // Reuse Context's attachment-to-message lowering to avoid duplicating
        // the per-variant rendering logic, then route the produced entries
        // into pending.
        let attachment_ctx =
            Context::default().add_attachments(attachments, Some(self.agent.model.clone()));
        for entry in attachment_ctx.messages {
            pending.user_input.push(entry);
        }

        Ok(conversation)
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{
        AgentId, AttachmentContent, Context, ContextMessage, ConversationId, FileInfo, ModelId,
        ProviderId, ToolKind,
    };
    use pretty_assertions::assert_eq;

    use super::*;

    struct MockService;

    #[async_trait::async_trait]
    impl AttachmentService for MockService {
        async fn attachments(&self, _url: &str) -> anyhow::Result<Vec<Attachment>> {
            Ok(Vec::new())
        }
    }

    impl crate::EnvironmentInfra for MockService {
        type Config = forge_config::ForgeConfig;

        fn get_environment(&self) -> forge_domain::Environment {
            use fake::{Fake, Faker};
            Faker.fake()
        }

        fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
            Ok(forge_config::ForgeConfig::default())
        }

        async fn update_environment(
            &self,
            _ops: Vec<forge_domain::ConfigOperation>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
            Default::default()
        }
    }

    fn fixture_agent_without_user_prompt() -> Agent {
        Agent::new(
            AgentId::from("test_agent"),
            ProviderId::OPENAI,
            ModelId::from("test-model"),
        )
    }

    fn fixture_conversation() -> Conversation {
        Conversation::new(ConversationId::default()).context(Context::default())
    }

    fn fixture_generator(agent: Agent, event: Event) -> UserPromptGenerator<MockService> {
        UserPromptGenerator::new(Arc::new(MockService), agent, event, chrono::Local::now())
    }

    #[tokio::test]
    async fn test_adds_context_as_droppable_message() {
        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("First Message").additional_context("Second Message");
        let conversation = fixture_conversation();
        let generator = fixture_generator(agent.clone(), event);

        let (conv, pending) = generator.generate(conversation).await.unwrap();

        assert!(
            conv.context.unwrap().messages.is_empty(),
            "canonical must stay untouched"
        );
        assert_eq!(pending.user_input.len(), 2);

        let task_message = pending.user_input.first().unwrap();
        assert_eq!(task_message.content().unwrap(), "First Message");
        assert!(!task_message.is_droppable());

        let context_message = pending.user_input.last().unwrap();
        assert_eq!(context_message.content().unwrap(), "Second Message");
        assert!(context_message.is_droppable());
    }

    #[tokio::test]
    async fn test_context_added_before_main_message() {
        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("First Message").additional_context("Second Message");
        let conversation = fixture_conversation();
        let generator = fixture_generator(agent.clone(), event);

        let (_, pending) = generator.generate(conversation).await.unwrap();

        assert_eq!(pending.user_input.len(), 2);
        assert_eq!(pending.user_input[0].content().unwrap(), "First Message");
        assert_eq!(pending.user_input[1].content().unwrap(), "Second Message");
    }

    #[tokio::test]
    async fn test_no_context_only_main_message() {
        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("Simple task");
        let conversation = fixture_conversation();
        let generator = fixture_generator(agent.clone(), event);

        let (_, pending) = generator.generate(conversation).await.unwrap();

        assert_eq!(pending.user_input.len(), 1);
        assert_eq!(pending.user_input[0].content().unwrap(), "Simple task");
    }

    #[tokio::test]
    async fn test_empty_event_no_message_added() {
        let agent = fixture_agent_without_user_prompt();
        let event = Event::empty();
        let conversation = fixture_conversation();
        let generator = fixture_generator(agent.clone(), event);

        let (_, pending) = generator.generate(conversation).await.unwrap();

        assert!(pending.user_input.is_empty());
        assert!(pending.continuation.is_empty());
    }

    #[tokio::test]
    async fn test_raw_content_preserved_in_message() {
        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("Task text");
        let conversation = fixture_conversation();
        let generator = fixture_generator(agent.clone(), event);

        let (_, pending) = generator.generate(conversation).await.unwrap();
        let message = pending.user_input.first().unwrap();

        if let ContextMessage::Text(text_msg) = &**message {
            assert!(text_msg.raw_content.is_some());
            let raw = text_msg.raw_content.as_ref().unwrap();
            assert_eq!(raw.as_user_prompt().unwrap().as_str(), "Task text");
        } else {
            panic!("expected TextMessage");
        }
    }

    /// The canonical invariant: `generate` leaves `conversation.context`
    /// byte-identical to its input — every new message goes into pending.
    #[tokio::test]
    async fn test_generate_leaves_canonical_untouched() {
        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("New user message");
        let conversation = Conversation::new(ConversationId::default()).context(
            Context::default()
                .add_message(ContextMessage::system("system"))
                .add_message(ContextMessage::user("prior turn", None)),
        );
        let before = conversation.context.clone();
        let generator = fixture_generator(agent.clone(), event);

        let (after, pending) = generator.generate(conversation).await.unwrap();

        assert_eq!(
            after.context, before,
            "canonical must not change as a result of generate()"
        );
        assert_eq!(pending.user_input.len(), 1);
    }

    #[tokio::test]
    async fn test_attachments_tracked_as_read_operations() {
        // Setup - Create a service that returns file attachments
        struct MockServiceWithFiles;

        impl crate::EnvironmentInfra for MockServiceWithFiles {
            type Config = forge_config::ForgeConfig;
            fn get_environment(&self) -> forge_domain::Environment {
                use fake::{Fake, Faker};
                Faker.fake()
            }
            fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
                Ok(forge_config::ForgeConfig::default())
            }
            async fn update_environment(
                &self,
                _ops: Vec<forge_domain::ConfigOperation>,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            fn get_env_var(&self, _key: &str) -> Option<String> {
                None
            }
            fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
                Default::default()
            }
        }

        #[async_trait::async_trait]
        impl AttachmentService for MockServiceWithFiles {
            async fn attachments(&self, _url: &str) -> anyhow::Result<Vec<Attachment>> {
                Ok(vec![
                    Attachment {
                        path: "/test/file1.rs".to_string(),
                        content: AttachmentContent::FileContent {
                            content: "fn main() {}".to_string(),
                            info: FileInfo::new(1, 1, 1, "hash1".to_string()),
                        },
                    },
                    Attachment {
                        path: "/test/file2.rs".to_string(),
                        content: AttachmentContent::FileContent {
                            content: "fn test() {}".to_string(),
                            info: FileInfo::new(1, 1, 1, "hash2".to_string()),
                        },
                    },
                ])
            }
        }

        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("Task with @[/test/file1.rs] and @[/test/file2.rs]");
        let conversation = Conversation::new(ConversationId::default());
        let generator = UserPromptGenerator::new(
            Arc::new(MockServiceWithFiles),
            agent.clone(),
            event,
            chrono::Local::now(),
        );

        // Execute
        let (actual, _pending) = generator.generate(conversation).await.unwrap();

        // Assert - Both files should be tracked as read operations
        let file1_op = actual.metrics.file_operations.get("/test/file1.rs");
        let file2_op = actual.metrics.file_operations.get("/test/file2.rs");

        assert!(file1_op.is_some(), "file1.rs should be tracked in metrics");
        assert!(file2_op.is_some(), "file2.rs should be tracked in metrics");

        // Verify the operation is marked as Read
        let file1_metrics = file1_op.unwrap();
        assert_eq!(
            file1_metrics.tool,
            ToolKind::Read,
            "file1.rs should be tracked as Read operation"
        );
        assert!(
            file1_metrics.content_hash.is_some(),
            "file1.rs should have content hash"
        );

        let file2_metrics = file2_op.unwrap();
        assert_eq!(
            file2_metrics.tool,
            ToolKind::Read,
            "file2.rs should be tracked as Read operation"
        );
        assert!(
            file2_metrics.content_hash.is_some(),
            "file2.rs should have content hash"
        );

        // Verify both files are in files_accessed (since they are Read operations)
        assert!(
            actual.metrics.files_accessed.contains("/test/file1.rs"),
            "file1.rs should be in files_accessed"
        );
        assert!(
            actual.metrics.files_accessed.contains("/test/file2.rs"),
            "file2.rs should be in files_accessed"
        );
    }

    #[tokio::test]
    async fn test_todos_injected_on_resume() {
        // Setup - Simple mock that returns no attachments
        struct MockServiceWithTodos;

        impl crate::EnvironmentInfra for MockServiceWithTodos {
            type Config = forge_config::ForgeConfig;
            fn get_environment(&self) -> forge_domain::Environment {
                use fake::{Fake, Faker};
                Faker.fake()
            }
            fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
                Ok(forge_config::ForgeConfig::default())
            }
            async fn update_environment(
                &self,
                _ops: Vec<forge_domain::ConfigOperation>,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            fn get_env_var(&self, _key: &str) -> Option<String> {
                None
            }
            fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
                Default::default()
            }
        }

        #[async_trait::async_trait]
        impl AttachmentService for MockServiceWithTodos {
            async fn attachments(&self, _url: &str) -> anyhow::Result<Vec<Attachment>> {
                Ok(Vec::new())
            }
        }

        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("Continue working");

        // Create a conversation with existing context (simulating resume) and todos
        // stored in metrics
        let conversation = Conversation::new(ConversationId::generate())
            .context(
                Context::default()
                    .add_message(ContextMessage::system("System message"))
                    .add_message(ContextMessage::user("Previous task", None)),
            )
            .metrics(Metrics::default().todos(vec![
                Todo::new("Task 1").status(TodoStatus::Completed),
                Todo::new("Task 2").status(TodoStatus::InProgress),
                Todo::new("Task 3").status(TodoStatus::Pending),
            ]));

        let generator = UserPromptGenerator::new(
            Arc::new(MockServiceWithTodos),
            agent.clone(),
            event,
            chrono::Local::now(),
        );

        // Execute
        let (actual, pending) = generator.generate(conversation).await.unwrap();

        // Assert - canonical stays at 2 messages (system + previous user);
        // new user message and todo list land in pending.
        let canonical = actual.context.unwrap().messages;
        assert_eq!(canonical.len(), 2);
        assert_eq!(canonical[0].content().unwrap(), "System message");
        assert_eq!(canonical[1].content().unwrap(), "Previous task");

        assert_eq!(pending.user_input.len(), 2);
        assert_eq!(pending.user_input[0].content().unwrap(), "Continue working");

        let todo_message = &pending.user_input[1];
        assert!(
            todo_message.is_droppable(),
            "Todo message should be droppable"
        );
        let todo_content = todo_message.content().unwrap();
        assert!(
            todo_content.contains("Current task list:"),
            "Should contain task list header"
        );
        assert!(
            todo_content.contains("[DONE] Task 1"),
            "Should contain completed task"
        );
        assert!(
            todo_content.contains("[IN_PROGRESS] Task 2"),
            "Should contain in-progress task"
        );
        assert!(
            todo_content.contains("[PENDING] Task 3"),
            "Should contain pending task"
        );
    }

    #[tokio::test]
    async fn test_todos_not_injected_on_new_conversation() {
        // Setup - Simple mock with no attachments
        struct MockServiceNoTodos;

        impl crate::EnvironmentInfra for MockServiceNoTodos {
            type Config = forge_config::ForgeConfig;
            fn get_environment(&self) -> forge_domain::Environment {
                use fake::{Fake, Faker};
                Faker.fake()
            }
            fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
                Ok(forge_config::ForgeConfig::default())
            }
            async fn update_environment(
                &self,
                _ops: Vec<forge_domain::ConfigOperation>,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            fn get_env_var(&self, _key: &str) -> Option<String> {
                None
            }
            fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
                Default::default()
            }
        }

        #[async_trait::async_trait]
        impl AttachmentService for MockServiceNoTodos {
            async fn attachments(&self, _url: &str) -> anyhow::Result<Vec<Attachment>> {
                Ok(Vec::new())
            }
        }

        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("First task");

        // Create a new conversation (no existing context, no todos)
        let conversation = Conversation::new(ConversationId::generate());

        let generator = UserPromptGenerator::new(
            Arc::new(MockServiceNoTodos),
            agent.clone(),
            event,
            chrono::Local::now(),
        );

        // Execute
        let (actual, pending) = generator.generate(conversation).await.unwrap();

        // Assert - canonical is empty; user message lands in pending with
        // no todo injection (new conversation, nothing to resume).
        let canonical = actual.context.unwrap_or_default().messages;
        assert!(canonical.is_empty(), "canonical untouched for new conv");
        assert_eq!(pending.user_input.len(), 1, "only the new user message");
        assert_eq!(pending.user_input[0].content().unwrap(), "First task");
    }
}
