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

    /// Sets the user prompt in the context based on agent configuration and
    /// event data
    pub async fn add_user_prompt(
        &self,
        conversation: Conversation,
    ) -> anyhow::Result<Conversation> {
        // Check if this is a resume BEFORE adding new messages
        let is_resume = conversation
            .context
            .as_ref()
            .map(|ctx| ctx.messages.iter().any(|msg| msg.has_role(Role::User)))
            .unwrap_or(false);

        let (conversation, content) = self.add_rendered_message(conversation).await?;
        let conversation = if is_resume {
            self.add_todos_on_resume(conversation)?
        } else {
            conversation
        };
        let conversation = self.add_additional_context(conversation).await?;
        let conversation = if let Some(content) = content {
            self.add_attachments(conversation, &content).await?
        } else {
            conversation
        };

        Ok(conversation)
    }

    /// Adds existing todos as a user message when resuming a conversation
    fn add_todos_on_resume(&self, mut conversation: Conversation) -> anyhow::Result<Conversation> {
        let mut context = conversation.context.take().unwrap_or_default();

        // Load existing todos from session metrics
        let todos = conversation.metrics.todos.clone();

        if !todos.is_empty() {
            // Format todos as markdown checklist
            let todo_content = self.format_todos_as_markdown(&todos);

            // Add as a droppable user message after the new task
            let todo_message = TextMessage {
                role: Role::User,
                content: todo_content,
                raw_content: None,
                tool_calls: None,
                thought_signature: None,
                reasoning_details: None,
                model: Some(self.agent.model.clone()),
                droppable: true, // Droppable so it can be removed during context compression
                phase: None,
            };
            context = context.add_message(ContextMessage::Text(todo_message));
        }

        Ok(conversation.context(context))
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

    /// Adds additional context (piped input) as a droppable user message
    async fn add_additional_context(
        &self,
        mut conversation: Conversation,
    ) -> anyhow::Result<Conversation> {
        let mut context = conversation.context.take().unwrap_or_default();

        if let Some(piped_input) = &self.event.additional_context {
            let piped_message = TextMessage {
                role: Role::User,
                content: piped_input.clone(),
                raw_content: None,
                tool_calls: None,
                thought_signature: None,
                reasoning_details: None,
                model: Some(self.agent.model.clone()),
                droppable: true, // Piped input is droppable
                phase: None,
            };
            context = context.add_message(ContextMessage::Text(piped_message));
        }

        Ok(conversation.context(context))
    }

    /// Renders the user message content and adds it to the conversation
    /// Returns the conversation and the rendered content for attachment parsing
    async fn add_rendered_message(
        &self,
        mut conversation: Conversation,
    ) -> anyhow::Result<(Conversation, Option<String>)> {
        let mut context = conversation.context.take().unwrap_or_default();
        let event_value = self.event.value.clone();
        let template_engine = TemplateEngine::default();

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

            // Check if context already contains user messages to determine if it's feedback
            let has_user_messages = context.messages.iter().any(|msg| msg.has_role(Role::User));

            if has_user_messages {
                event_context = event_context.into_feedback();
            } else {
                event_context = event_context.into_task();
            }

            debug!(event_context = ?event_context, "Event context");

            // Render the command first.
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

            // Inject terminal context into the event context when available.
            let event_context =
                match TerminalContextService::new(self.services.clone()).get_terminal_context() {
                    Some(ctx) => event_context.terminal_context(Some(ctx)),
                    None => event_context,
                };

            // Render the event value into agent's user prompt template.
            Some(
                template_engine.render_template(
                    Template::new(user_prompt.template.as_str()),
                    &event_context,
                )?,
            )
        } else {
            // Use the raw event value as content if no user_prompt is provided
            event_value
                .as_ref()
                .and_then(|v| v.as_user_prompt().map(|p| p.deref().to_owned()))
        };

        if let Some(content) = &content {
            // Create User Message
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
            context = context.add_message(ContextMessage::Text(message));
        }

        Ok((conversation.context(context), content))
    }

    /// Parses and adds attachments to the conversation based on the provided
    /// content
    async fn add_attachments(
        &self,
        mut conversation: Conversation,
        content: &str,
    ) -> anyhow::Result<Conversation> {
        let mut context = conversation.context.take().unwrap_or_default();

        // Parse Attachments (do NOT parse piped input for attachments)
        let attachments = self.services.attachments(content).await?;

        // Track file attachments as read operations in metrics
        let mut metrics = conversation.metrics.clone();
        for attachment in &attachments {
            // Only track file content attachments (not images or directory listings).
            // Use the raw content_hash (computed before line-numbering) so that the
            // external-change detector, which hashes the raw file on disk, sees a
            // matching hash and does not raise a false "modified externally" warning.
            if let AttachmentContent::FileContent { info, .. } = &attachment.content {
                metrics = metrics.insert(
                    attachment.path.clone(),
                    FileOperation::new(ToolKind::Read)
                        .content_hash(Some(info.content_hash.clone())),
                );
            }
        }
        conversation.metrics = metrics;

        context = context.add_attachments(attachments, Some(self.agent.model.clone()));

        Ok(conversation.context(context))
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

        let actual = generator.add_user_prompt(conversation).await.unwrap();

        let messages = actual.context.unwrap().messages;
        assert_eq!(
            messages.len(),
            2,
            "Should have context message and main message"
        );

        // First message should be the context (droppable)
        let task_message = messages.first().unwrap();
        assert_eq!(task_message.content().unwrap(), "First Message");
        assert!(
            !task_message.is_droppable(),
            "Context message should be droppable"
        );

        // Second message should not be droppable
        let context_message = messages.last().unwrap();
        assert_eq!(context_message.content().unwrap(), "Second Message");
        assert!(
            context_message.is_droppable(),
            "Main message should not be droppable"
        );
    }

    #[tokio::test]
    async fn test_context_added_before_main_message() {
        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("First Message").additional_context("Second Message");
        let conversation = fixture_conversation();
        let generator = fixture_generator(agent.clone(), event);

        let actual = generator.add_user_prompt(conversation).await.unwrap();

        let messages = actual.context.unwrap().messages;
        assert_eq!(messages.len(), 2);

        // Verify order: main message first, then additional context
        assert_eq!(messages[0].content().unwrap(), "First Message");
        assert_eq!(messages[1].content().unwrap(), "Second Message");
    }

    #[tokio::test]
    async fn test_no_context_only_main_message() {
        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("Simple task");
        let conversation = fixture_conversation();
        let generator = fixture_generator(agent.clone(), event);

        let actual = generator.add_user_prompt(conversation).await.unwrap();

        let messages = actual.context.unwrap().messages;
        assert_eq!(messages.len(), 1, "Should only have the main message");
        assert_eq!(messages[0].content().unwrap(), "Simple task");
    }

    #[tokio::test]
    async fn test_empty_event_no_message_added() {
        let agent = fixture_agent_without_user_prompt();
        let event = Event::empty();
        let conversation = fixture_conversation();
        let generator = fixture_generator(agent.clone(), event);

        let actual = generator.add_user_prompt(conversation).await.unwrap();

        let messages = actual.context.unwrap().messages;
        assert_eq!(
            messages.len(),
            0,
            "Should not add any message for empty event"
        );
    }

    #[tokio::test]
    async fn test_raw_content_preserved_in_message() {
        let agent = fixture_agent_without_user_prompt();
        let event = Event::new("Task text");
        let conversation = fixture_conversation();
        let generator = fixture_generator(agent.clone(), event);

        let actual = generator.add_user_prompt(conversation).await.unwrap();

        let messages = actual.context.unwrap().messages;
        let message = messages.first().unwrap();

        if let ContextMessage::Text(text_msg) = &**message {
            assert!(
                text_msg.raw_content.is_some(),
                "Raw content should be preserved"
            );
            let raw = text_msg.raw_content.as_ref().unwrap();
            assert_eq!(raw.as_user_prompt().unwrap().as_str(), "Task text");
        } else {
            panic!("Expected TextMessage");
        }
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
        let actual = generator.add_user_prompt(conversation).await.unwrap();

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
        let actual = generator.add_user_prompt(conversation).await.unwrap();

        // Assert - Should have system, previous user, new user message, and todo list
        let messages = actual.context.unwrap().messages;
        assert_eq!(messages.len(), 4, "Should have 4 messages");

        // First is system message
        assert_eq!(messages[0].content().unwrap(), "System message");

        // Second is previous user task
        assert_eq!(messages[1].content().unwrap(), "Previous task");

        // Third is the new user message
        assert_eq!(messages[2].content().unwrap(), "Continue working");

        // Fourth should be the todo list (droppable)
        let todo_message = &messages[3];
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
        let actual = generator.add_user_prompt(conversation).await.unwrap();

        // Assert - Should only have the user message, no todos
        let messages = actual.context.unwrap().messages;
        assert_eq!(messages.len(), 1, "Should only have user message");
        assert_eq!(messages[0].content().unwrap(), "First task");
    }
}
