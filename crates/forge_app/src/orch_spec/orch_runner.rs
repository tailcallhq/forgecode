use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use forge_domain::{
    Attachment, ChatCompletionMessage, ChatResponse, Conversation, ConversationId, Environment,
    Event, Hook, ProviderId, ToolCallFull, ToolErrorTracker, ToolResult,
};
use handlebars::{Handlebars, no_escape};
use include_dir::{Dir, include_dir};
use tokio::sync::Mutex;

pub use super::orch_setup::TestContext;
use crate::app::build_template_config;
use crate::apply_tunable_parameters::ApplyTunableParameters;
use crate::hooks::{DoomLoopDetector, PendingTodosHandler};
use crate::init_conversation_metrics::InitConversationMetrics;
use crate::orch::Orchestrator;
use crate::set_conversation_id::SetConversationId;
use crate::system_prompt::SystemPrompt;
use crate::user_prompt::UserPromptGenerator;
use crate::{
    AgentExt, AgentService, AttachmentService, EnvironmentInfra, ShellOutput, ShellService,
    SkillFetchService, TemplateService,
};

static TEMPLATE_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../templates");

pub struct Runner {
    hb: Handlebars<'static>,
    // History of all the updates made to the conversation
    conversation_history: Mutex<Vec<Conversation>>,

    // Tool call requests and the mock responses
    test_tool_calls: Mutex<VecDeque<(ToolCallFull, ToolResult)>>,

    // Mock completions from the LLM (Each value is produced as an event in the stream)
    test_completions: Mutex<VecDeque<ChatCompletionMessage>>,

    // Mock shell command outputs
    test_shell_outputs: Mutex<VecDeque<ShellOutput>>,

    attachments: Vec<Attachment>,
    config: forge_config::ForgeConfig,
    env: Environment,
}

impl Runner {
    fn new(setup: &TestContext) -> Self {
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        hb.register_escape_fn(no_escape);

        // Register all embedded templates from the templates directory
        forge_embed::register_templates(&mut hb, &TEMPLATE_DIR);
        for (name, tpl) in &setup.templates {
            hb.register_template_string(name, tpl).unwrap();
        }

        Self {
            hb,
            attachments: setup.attachments.clone(),
            config: setup.config.clone(),
            env: setup.env.clone(),
            conversation_history: Mutex::new(Vec::new()),
            test_tool_calls: Mutex::new(VecDeque::from(setup.mock_tool_call_responses.clone())),
            test_completions: Mutex::new(VecDeque::from(setup.mock_assistant_responses.clone())),
            test_shell_outputs: Mutex::new(VecDeque::from(setup.mock_shell_outputs.clone())),
        }
    }

    // Returns the conversation history
    async fn get_history(&self) -> Vec<Conversation> {
        self.conversation_history.lock().await.clone()
    }

    pub async fn run(setup: &mut TestContext, event: Event) -> anyhow::Result<()> {
        const LIMIT: usize = 1024;
        let (tx, mut rx) = tokio::sync::mpsc::channel::<anyhow::Result<ChatResponse>>(LIMIT);
        let handle = tokio::spawn(async move {
            let mut responses = Vec::new();

            while let Some(item) = rx.recv().await {
                // Simulate what the real UI does: acknowledge ToolCallStart so
                // the orchestrator can proceed with tool execution.
                if let Ok(ChatResponse::ToolCallStart { ref notifier, .. }) = item {
                    notifier.notify_one();
                }
                responses.push(item);
            }

            responses
        });

        let services = Arc::new(Runner::new(setup));
        // setup the conversation
        let conversation = Conversation::new(ConversationId::generate()).title(setup.title.clone());

        let agent = setup.agent.clone();
        let system_tools = setup.tools.clone();
        let agent = agent.apply_config(&setup.config).model(setup.model.clone());

        // Render system prompt into context.
        let conversation = SystemPrompt::new(services.clone(), setup.env.clone(), agent.clone())
            .files(setup.files.clone())
            .tool_definitions(system_tools.clone())
            .max_extensions(setup.config.max_extensions)
            .template_config(build_template_config(&setup.config))
            .add_system_message(conversation)
            .await?;

        // Render user prompt into context.
        let conversation = UserPromptGenerator::new(
            services.clone(),
            agent.clone(),
            event.clone(),
            setup.current_time,
        )
        .add_user_prompt(conversation)
        .await?;

        let conversation = InitConversationMetrics::new(setup.current_time).apply(conversation);
        // Apply initial metrics (including todos) if provided by the test
        let conversation = if let Some(ref metrics) = setup.initial_metrics {
            conversation.metrics(metrics.clone())
        } else {
            conversation
        };
        let conversation =
            ApplyTunableParameters::new(agent.clone(), system_tools.clone()).apply(conversation);
        let conversation = SetConversationId.apply(conversation);

        let orch = Orchestrator::new(services.clone(), conversation, agent, setup.config.clone())
            .error_tracker(ToolErrorTracker::new(3))
            .tool_definitions(system_tools)
            .hook(Arc::new(
                Hook::default()
                    .on_request(DoomLoopDetector::default())
                    .on_end(PendingTodosHandler::new()),
            ))
            .sender(tx);

        let (mut orch, runner) = (orch, services);

        let result = orch.run().await;
        drop(orch);

        let chat_responses = handle.await?;

        setup.output.chat_responses.extend(chat_responses);
        setup
            .output
            .conversation_history
            .extend(runner.get_history().await);

        result
    }
}

#[async_trait::async_trait]
impl AgentService for Runner {
    async fn chat_agent(
        &self,
        _id: &forge_domain::ModelId,
        context: forge_domain::Context,
        _provider_id: Option<ProviderId>,
    ) -> forge_domain::ResultStream<ChatCompletionMessage, anyhow::Error> {
        let mut responses = self.test_completions.lock().await;

        if let Some(message) = responses.pop_front() {
            Ok(Box::pin(tokio_stream::iter(std::iter::once(Ok(message)))))
        } else {
            let total_messages = context.messages.len();
            let last_message = context.messages.last();
            panic!(
                "No mock response found. Total Messages: {total_messages}. Last Message: {last_message:#?}"
            )
        }
    }

    async fn call(
        &self,
        _: &forge_domain::Agent,
        _: &forge_domain::ToolCallContext,
        test_call: forge_domain::ToolCallFull,
    ) -> forge_domain::ToolResult {
        let name = test_call.name.clone();
        let mut guard = self.test_tool_calls.lock().await;
        for (id, (call, result)) in guard.iter().enumerate() {
            if call.call_id == test_call.call_id {
                let result = result.clone();
                guard.remove(id);
                return result;
            }
        }

        panic!("No mock tool call not found: {name}")
    }

    async fn update(&self, conversation: Conversation) -> anyhow::Result<()> {
        self.conversation_history.lock().await.push(conversation);
        Ok(())
    }
}

#[async_trait::async_trait]
impl TemplateService for Runner {
    async fn register_template(&self, _path: std::path::PathBuf) -> anyhow::Result<()> {
        unimplemented!()
    }

    async fn render_template<V: serde::Serialize + Send + Sync>(
        &self,
        template: forge_domain::Template<V>,
        object: &V,
    ) -> anyhow::Result<String> {
        Ok(self.hb.render_template(&template.template, object)?)
    }
}

#[async_trait::async_trait]
impl AttachmentService for Runner {
    async fn attachments(&self, _url: &str) -> anyhow::Result<Vec<forge_domain::Attachment>> {
        Ok(self.attachments.clone())
    }
}

#[async_trait::async_trait]
impl SkillFetchService for Runner {
    async fn fetch_skill(&self, _skill_name: String) -> anyhow::Result<forge_domain::Skill> {
        unimplemented!("SkillFetchService not implemented for test Runner")
    }

    async fn list_skills(&self) -> anyhow::Result<Vec<forge_domain::Skill>> {
        Ok(vec![])
    }
}

#[async_trait::async_trait]
impl ShellService for Runner {
    async fn execute(
        &self,
        _command: String,
        _cwd: std::path::PathBuf,
        _keep_ansi: bool,
        _silent: bool,
        _env_vars: Option<Vec<String>>,
        _description: Option<String>,
    ) -> anyhow::Result<ShellOutput> {
        let mut outputs = self.test_shell_outputs.lock().await;
        if let Some(output) = outputs.pop_front() {
            Ok(output)
        } else {
            Ok(ShellOutput {
                output: forge_domain::CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    command: String::new(),
                    exit_code: Some(1),
                },
                shell: "/bin/bash".to_string(),
                description: None,
            })
        }
    }
}

impl EnvironmentInfra for Runner {
    type Config = forge_config::ForgeConfig;

    fn get_env_var(&self, _key: &str) -> Option<String> {
        None
    }

    fn get_env_vars(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn get_environment(&self) -> forge_domain::Environment {
        self.env.clone()
    }

    fn get_config(&self) -> anyhow::Result<Self::Config> {
        Ok(self.config.clone())
    }

    async fn update_environment(
        &self,
        _ops: Vec<forge_domain::ConfigOperation>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
