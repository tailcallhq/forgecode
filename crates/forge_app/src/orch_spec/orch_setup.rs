use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Local};
use derive_setters::Setters;
use forge_config::ForgeConfig;
use forge_domain::{
    Agent, AgentId, Attachment, ChatCompletionMessage, ChatResponse, Conversation, Environment,
    Event, File, MessageEntry, Metrics, ModelId, ProviderId, Role, Template, ToolCallFull,
    ToolDefinition, ToolResult,
};

use crate::ShellOutput;
use crate::orch_spec::orch_runner::Runner;

// User prompt
const USER_PROMPT: &str = r#"
  <{{event.name}}>{{event.value}}</{{event.name}}>
  <system_date>{{current_date}}</system_date>
"#;

#[derive(Setters)]
#[setters(into)]
pub struct TestContext {
    pub mock_tool_call_responses: Vec<(ToolCallFull, ToolResult)>,
    pub mock_assistant_responses: Vec<ChatCompletionMessage>,
    pub mock_shell_outputs: Vec<ShellOutput>,
    pub templates: HashMap<String, String>,
    pub files: Vec<File>,
    pub env: Environment,
    pub current_time: DateTime<Local>,
    pub title: Option<String>,
    pub model: ModelId,
    pub attachments: Vec<Attachment>,

    // Initial metrics to apply to the conversation
    pub initial_metrics: Option<Metrics>,

    // Final output of the test is store in the context
    pub output: TestOutput,
    pub agent: Agent,
    pub tools: Vec<ToolDefinition>,
    /// ForgeConfig used to populate TemplateConfig for
    /// system prompt rendering in tests.
    pub config: ForgeConfig,
}

impl Default for TestContext {
    fn default() -> Self {
        Self {
            model: ModelId::new("openai/gpt-1"),
            output: TestOutput::default(),
            current_time: Local::now(),
            mock_assistant_responses: Default::default(),
            mock_tool_call_responses: Default::default(),
            mock_shell_outputs: Default::default(),
            templates: Default::default(),
            files: Default::default(),
            attachments: Default::default(),
            initial_metrics: None,
            env: Environment {
                os: "MacOS".to_string(),
                cwd: PathBuf::from("/Users/tushar"),
                home: Some(PathBuf::from("/Users/tushar")),
                shell: "bash".to_string(),
                base_path: PathBuf::from("/Users/tushar/projects"),
            },
            config: ForgeConfig::default()
                .tool_supported(true)
                .max_extensions(15),
            title: Some("test-conversation".into()),
            agent: Agent::new(
                AgentId::new("forge"),
                ProviderId::ANTHROPIC,
                ModelId::new("claude-3-5-sonnet-20241022"),
            )
            .system_prompt(Template::new("You are Forge"))
            .user_prompt(Template::new(USER_PROMPT))
            .tools(vec![("fs_read").into(), ("fs_write").into()]),
            tools: vec![
                ToolDefinition::new("fs_read"),
                ToolDefinition::new("fs_write"),
            ],
        }
    }
}

impl TestContext {
    pub async fn run(&mut self, event: impl AsRef<str>) -> anyhow::Result<()> {
        self.run_event(Event::new(event.as_ref())).await
    }

    pub async fn run_event(&mut self, event: impl Into<Event>) -> anyhow::Result<()> {
        Runner::run(self, event.into()).await
    }
}

// The final output produced after running the orchestrator to completion
#[derive(Default, Debug)]
pub struct TestOutput {
    pub conversation_history: Vec<Conversation>,
    pub chat_responses: Vec<anyhow::Result<ChatResponse>>,
}

impl TestOutput {
    pub fn system_messages(&self) -> Option<Vec<&str>> {
        self.conversation_history
            .last()
            .and_then(|c| c.context.as_ref())
            .and_then(|c| {
                c.messages
                    .iter()
                    .filter(|c| c.has_role(Role::System))
                    .map(|m| m.content())
                    .collect()
            })
    }

    pub fn context_messages(&self) -> Vec<MessageEntry> {
        self.conversation_history
            .last()
            .and_then(|c| c.context.as_ref())
            .map(|c| c.messages.clone())
            .clone()
            .unwrap_or_default()
    }

    pub fn tools(&self) -> Vec<ToolDefinition> {
        self.conversation_history
            .last()
            .and_then(|c| c.context.as_ref())
            .map(|c| c.tools.clone())
            .clone()
            .unwrap_or_default()
    }
}
