use std::sync::Arc;

use anyhow::Result;
use forge_domain::*;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    AppConfigService, EnvironmentInfra, FileDiscoveryService, ProviderService, TemplateEngine,
    TerminalContextService,
};

/// Response struct for shell command generation using JSON format
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(title = "shell_command")]
pub struct ShellCommandResponse {
    /// The generated shell command
    pub command: String,
}

/// CommandGenerator handles shell command generation from natural language
pub struct CommandGenerator<S> {
    services: Arc<S>,
}

impl<S> CommandGenerator<S>
where
    S: EnvironmentInfra<Config = forge_config::ForgeConfig>
        + FileDiscoveryService
        + ProviderService
        + AppConfigService,
{
    /// Creates a new CommandGenerator instance with the provided services.
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }

    /// Generates a shell command from a natural language prompt.
    ///
    /// Terminal context is read automatically from the `_FORGE_TERM_COMMANDS`,
    /// `_FORGE_TERM_EXIT_CODES`, and `_FORGE_TERM_TIMESTAMPS` environment
    /// variables exported by the zsh plugin, and included in the user
    /// prompt so the LLM can reference recent commands, exit codes, and
    /// timestamps.
    pub async fn generate(&self, prompt: UserPrompt) -> Result<String> {
        // Get system information for context
        let env = self.services.get_environment();

        let files = self.services.list_current_directory().await?;

        let rendered_system_prompt = TemplateEngine::default().render(
            "forge-command-generator-prompt.md",
            &serde_json::json!({"env": env, "files": files}),
        )?;

        // Get required services and data - use suggest config if available,
        // otherwise fall back to default provider/model
        let (provider, model) = match self.services.get_suggest_config().await? {
            Some(config) => {
                let provider = self.services.get_provider(config.provider).await?;
                (provider, config.model)
            }
            None => {
                let provider_id = self.services.get_default_provider().await?;
                let provider = self.services.get_provider(provider_id).await?;
                let model = self.services.get_provider_model(Some(&provider.id)).await?;
                (provider, model)
            }
        };

        // Build user prompt with task, optionally including terminal context.
        use forge_template::Element;
        let task_elm = Element::new("task").text(prompt.as_str());
        let terminal_service = TerminalContextService::new(self.services.clone());
        let user_content = match terminal_service.get_terminal_context() {
            Some(ctx) => {
                let terminal_elm =
                    Element::new("terminal_context").append(ctx.commands.iter().map(|cmd| {
                        Element::new("command")
                            .attr("exit_code", cmd.exit_code.to_string())
                            .text(&cmd.command)
                    }));
                format!("{}\n\n{}", terminal_elm.render(), task_elm.render())
            }
            None => task_elm.render(),
        };

        // Create context with system and user prompts
        let ctx = self.create_context(rendered_system_prompt, user_content, &model);

        // Send message to LLM
        let stream = self.services.chat(&model, ctx, provider).await?;
        let message = stream.into_full(false).await?;

        // Parse the structured JSON response
        let response: ShellCommandResponse =
            serde_json::from_str(&message.content).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to parse shell command response: {}. Response: {}",
                    e,
                    message.content
                )
            })?;

        Ok(response.command)
    }

    /// Creates a context with system and user messages for the LLM
    fn create_context(
        &self,
        system_prompt: String,
        user_content: String,
        model: &ModelId,
    ) -> Context {
        // Generate JSON schema from the response struct
        let schema = schemars::schema_for!(ShellCommandResponse);

        Context::default()
            .add_message(ContextMessage::system(system_prompt))
            .add_message(ContextMessage::user(user_content, Some(model.clone())))
            .response_format(ResponseFormat::JsonSchema(Box::new(schema)))
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{
        AuthCredential, AuthDetails, AuthMethod, ChatCompletionMessage, Content, FinishReason,
        ModelSource, ProviderId, ProviderResponse, ResultStream, Role,
    };
    use tokio::sync::Mutex;
    use url::Url;

    use super::*;
    use crate::Walker;

    struct MockServices {
        files: Vec<(String, bool)>,
        response: Arc<Mutex<Option<String>>>,
        captured_context: Arc<Mutex<Option<Context>>>,
        environment: Environment,
        env_vars: std::collections::BTreeMap<String, String>,
    }

    impl MockServices {
        fn new(response: &str, files: Vec<(&str, bool)>) -> Arc<Self> {
            use fake::{Fake, Faker};
            let mut env: Environment = Faker.fake();
            // Override only the fields that appear in templates
            env.os = "macos".to_string();
            env.cwd = "/test/dir".into();
            env.shell = "/bin/bash".to_string();
            env.home = Some("/home/test".into());

            Arc::new(Self {
                files: files.into_iter().map(|(p, d)| (p.to_string(), d)).collect(),
                response: Arc::new(Mutex::new(Some(response.to_string()))),
                captured_context: Arc::new(Mutex::new(None)),
                environment: env,
                env_vars: std::collections::BTreeMap::new(),
            })
        }

        fn with_terminal_context(
            self: Arc<Self>,
            commands: &str,
            exit_codes: &str,
            timestamps: &str,
        ) -> Arc<Self> {
            let mut env_vars = self.env_vars.clone();
            env_vars.insert("_FORGE_TERM_COMMANDS".to_string(), commands.to_string());
            env_vars.insert("_FORGE_TERM_EXIT_CODES".to_string(), exit_codes.to_string());
            env_vars.insert("_FORGE_TERM_TIMESTAMPS".to_string(), timestamps.to_string());
            Arc::new(Self {
                files: self.files.clone(),
                response: self.response.clone(),
                captured_context: self.captured_context.clone(),
                environment: self.environment.clone(),
                env_vars,
            })
        }
    }

    impl EnvironmentInfra for MockServices {
        type Config = forge_config::ForgeConfig;

        fn get_environment(&self) -> Environment {
            self.environment.clone()
        }

        fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
            Ok(forge_config::ForgeConfig::default())
        }

        async fn update_environment(
            &self,
            _ops: Vec<forge_domain::ConfigOperation>,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }

        fn get_env_var(&self, key: &str) -> Option<String> {
            self.env_vars.get(key).cloned()
        }

        fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
            self.env_vars.clone()
        }
    }

    #[async_trait::async_trait]
    impl FileDiscoveryService for MockServices {
        async fn collect_files(&self, _walker: Walker) -> Result<Vec<File>> {
            Ok(self
                .files
                .iter()
                .map(|(path, is_dir)| File { path: path.clone(), is_dir: *is_dir })
                .collect())
        }

        async fn list_current_directory(&self) -> Result<Vec<File>> {
            let mut files: Vec<File> = self
                .files
                .iter()
                .map(|(path, is_dir)| File { path: path.clone(), is_dir: *is_dir })
                .collect();

            // Sort: directories first (alphabetically), then files (alphabetically)
            files.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.path.cmp(&b.path),
            });

            Ok(files)
        }
    }

    #[async_trait::async_trait]
    impl ProviderService for MockServices {
        async fn chat(
            &self,
            _id: &ModelId,
            context: Context,
            _provider: Provider<Url>,
        ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
            *self.captured_context.lock().await = Some(context);

            let response = self.response.lock().await.take().unwrap();
            let message = ChatCompletionMessage::assistant(Content::full(response))
                .finish_reason(FinishReason::Stop);
            Ok(Box::pin(tokio_stream::iter(std::iter::once(Ok(message)))))
        }

        async fn models(&self, _provider: Provider<Url>) -> Result<Vec<forge_domain::Model>> {
            Ok(vec![])
        }

        async fn get_provider(&self, _id: ProviderId) -> Result<Provider<Url>> {
            Ok(Provider {
                id: ProviderId::OPENAI,
                provider_type: Default::default(),
                response: Some(ProviderResponse::OpenAI),
                url: Url::parse("https://api.test.com").unwrap(),
                models: Some(ModelSource::Url(
                    Url::parse("https://api.test.com/models").unwrap(),
                )),
                auth_methods: vec![AuthMethod::ApiKey],
                url_params: vec![],
                credential: Some(AuthCredential {
                    id: ProviderId::OPENAI,
                    auth_details: AuthDetails::ApiKey("test-key".to_string().into()),
                    url_params: Default::default(),
                }),
                custom_headers: None,
            })
        }

        async fn get_all_providers(&self) -> Result<Vec<forge_domain::AnyProvider>> {
            Ok(vec![])
        }

        async fn upsert_credential(&self, _credential: AuthCredential) -> Result<()> {
            Ok(())
        }

        async fn remove_credential(&self, _id: &ProviderId) -> Result<()> {
            Ok(())
        }

        async fn migrate_env_credentials(&self) -> anyhow::Result<Option<MigrationResult>> {
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl AppConfigService for MockServices {
        async fn get_default_provider(&self) -> Result<ProviderId> {
            Ok(ProviderId::OPENAI)
        }

        async fn get_provider_model(
            &self,
            _provider_id: Option<&ProviderId>,
        ) -> anyhow::Result<ModelId> {
            Ok(ModelId::new("test-model"))
        }

        async fn get_commit_config(&self) -> Result<Option<forge_domain::ModelConfig>> {
            Ok(None)
        }

        async fn get_suggest_config(&self) -> Result<Option<forge_domain::ModelConfig>> {
            Ok(None)
        }

        async fn get_reasoning_effort(&self) -> Result<Option<forge_domain::Effort>> {
            Ok(None)
        }

        async fn update_config(&self, _ops: Vec<forge_domain::ConfigOperation>) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_generate_simple_command() {
        let fixture = MockServices::new(
            r#"{"command": "ls -la"}"#,
            vec![("file1.txt", false), ("file2.rs", false)],
        );
        let generator = CommandGenerator::new(fixture.clone());

        let actual = generator
            .generate(UserPrompt::from("list all files".to_string()))
            .await
            .unwrap();

        assert_eq!(actual, "ls -la");
        let captured_context = fixture.captured_context.lock().await.clone().unwrap();
        insta::assert_yaml_snapshot!(captured_context);
    }

    #[tokio::test]
    async fn test_generate_with_no_files() {
        let fixture = MockServices::new(r#"{"command": "pwd"}"#, vec![]);
        let generator = CommandGenerator::new(fixture.clone());

        let actual = generator
            .generate(UserPrompt::from("show current directory".to_string()))
            .await
            .unwrap();

        assert_eq!(actual, "pwd");
        let captured_context = fixture.captured_context.lock().await.clone().unwrap();
        insta::assert_yaml_snapshot!(captured_context);
    }

    #[tokio::test]
    async fn test_generate_with_shell_context() {
        let fixture = MockServices::new(
            r#"{"command": "cargo build --release"}"#,
            vec![("Cargo.toml", false)],
        )
        .with_terminal_context("cargo build", "101", "1700000000");
        let generator = CommandGenerator::new(fixture.clone());

        let actual = generator
            .generate(UserPrompt::from("fix the command I just ran".to_string()))
            .await
            .unwrap();

        assert_eq!(actual, "cargo build --release");
        let captured_context = fixture.captured_context.lock().await.clone().unwrap();
        let user_content = captured_context
            .messages
            .iter()
            .find(|m| m.has_role(Role::User))
            .expect("should have a user message")
            .content()
            .expect("user message should have content");
        assert!(user_content.contains("<terminal_context>"));
        assert!(user_content.contains("</terminal_context>"));
        assert!(user_content.contains("cargo build"));
        assert!(user_content.contains("<task>fix the command I just ran</task>"));
    }

    #[tokio::test]
    async fn test_generate_fails_when_missing_tag() {
        let fixture = MockServices::new(r#"{"invalid": "json"}"#, vec![]);
        let generator = CommandGenerator::new(fixture);

        let actual = generator
            .generate(UserPrompt::from("do something".to_string()))
            .await;

        assert!(actual.is_err());
        let error_msg = actual.unwrap_err().to_string();
        assert!(error_msg.contains("Failed to parse shell command response"));
    }
}
