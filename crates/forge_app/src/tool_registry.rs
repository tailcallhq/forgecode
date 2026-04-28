use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use console::style;
use forge_domain::{
    Agent, AgentId, AgentInput, ChatResponse, ChatResponseContent, Environment, InputModality,
    Model, SystemContext, TemplateConfig, ToolCallContext, ToolCallFull, ToolCatalog,
    ToolDefinition, ToolKind, ToolName, ToolOutput, ToolResult,
};
use forge_template::Element;
use futures::future::join_all;
use serde_json::{Map, Value, json};
use strum::IntoEnumIterator;
use tokio::time::timeout;

use crate::agent_executor::AgentExecutor;
use crate::dto::ToolsOverview;
use crate::error::Error;
use crate::fmt::content::FormatContent;
use crate::mcp_executor::McpExecutor;
use crate::tool_executor::ToolExecutor;
use crate::{
    AgentRegistry, EnvironmentInfra, McpService, PolicyService, ProviderService, Services,
    ToolResolver, WorkspaceService,
};

pub struct ToolRegistry<S> {
    tool_executor: ToolExecutor<S>,
    agent_executor: AgentExecutor<S>,
    mcp_executor: McpExecutor<S>,
    services: Arc<S>,
}

impl<S: Services + EnvironmentInfra<Config = forge_config::ForgeConfig>> ToolRegistry<S> {
    pub fn new(services: Arc<S>) -> Self {
        Self {
            services: services.clone(),
            tool_executor: ToolExecutor::new(services.clone()),
            agent_executor: AgentExecutor::new(services.clone()),
            mcp_executor: McpExecutor::new(services.clone()),
        }
    }

    async fn call_with_timeout<F, Fut>(
        &self,
        tool_name: &ToolName,
        future: F,
    ) -> anyhow::Result<ToolOutput>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<ToolOutput>>,
    {
        let tool_timeout = Duration::from_secs(self.services.get_config()?.tool_timeout_secs);
        timeout(tool_timeout, future())
            .await
            .context(Error::CallTimeout {
                timeout: tool_timeout.as_secs() / 60,
                tool_name: tool_name.clone(),
            })?
    }

    /// Check if a tool operation is allowed based on the workflow policies
    async fn check_tool_permission(
        &self,
        tool_input: &ToolCatalog,
        context: &ToolCallContext,
    ) -> anyhow::Result<bool> {
        let cwd = self.services.get_environment().cwd;
        let operation = tool_input.to_policy_operation(cwd.clone());
        if let Some(operation) = operation {
            let decision = self.services.check_operation_permission(&operation).await?;

            // Send custom policy message to the user when a policy file was created
            if let Some(policy_path) = decision.path {
                use forge_domain::TitleFormat;

                use crate::utils::format_display_path;
                context
                    .send_tool_input(
                        TitleFormat::debug("Permissions Update")
                            .sub_title(format_display_path(policy_path.as_path(), &cwd)),
                    )
                    .await?;
            }
            if !decision.allowed {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn call_inner(
        &self,
        agent: &Agent,
        input: ToolCallFull,
        context: &ToolCallContext,
    ) -> anyhow::Result<ToolOutput> {
        Self::validate_tool_call(agent, &input.name)?;

        tracing::info!(tool_name = %input.name, arguments = %input.arguments.clone().into_string(), "Executing tool call");
        let tool_name = input.name.clone();

        // First, try to call a Forge tool
        if ToolCatalog::contains(&input.name) {
            let tool_input: ToolCatalog = ToolCatalog::try_from(input)?;

            // Special handling for Task tool - delegate to AgentExecutor
            if let ToolCatalog::Task(task_input) = tool_input {
                let executor = self.agent_executor.clone();
                let session_id = task_input.session_id.clone();
                let agent_id = task_input.agent_id.clone();
                // Parse session_id into ConversationId if present
                let conversation_id = session_id
                    .map(|id| forge_domain::ConversationId::parse(&id))
                    .transpose()
                    .ok()
                    .flatten();
                // NOTE: Agents should not timeout
                let outputs = join_all(task_input.tasks.into_iter().map(|task| {
                    let agent_id = agent_id.clone();
                    let executor = executor.clone();
                    async move {
                        executor
                            .execute(AgentId::new(&agent_id), task, context, conversation_id)
                            .await
                    }
                }))
                .await
                .into_iter()
                .collect::<anyhow::Result<Vec<_>>>()?;
                return Ok(ToolOutput::from(outputs.into_iter()));
            }

            let env = self.services.get_environment();
            if let Some(content) = tool_input.to_content(&env) {
                context.send(content).await?;
            }

            // Check permissions before executing the tool (only in restricted mode)
            // This is done BEFORE the timeout to ensure permissions are never timed out
            let is_restricted = self.services.get_config()?.restricted;
            if is_restricted && self.check_tool_permission(&tool_input, context).await? {
                // Send formatted output message for policy denial
                context
                    .send(forge_domain::TitleFormat::error("Permission Denied"))
                    .await?;

                return Ok(ToolOutput::text(
                    Element::new("permission_denied")
                        .cdata("User has denied the permission to execute this tool"),
                ));
            }

            // Validate tool modality support before execution
            // Only resolve the current model when modality validation is needed.
            if matches!(&tool_input, ToolCatalog::Read(input) if Self::has_image_extension(&input.file_path))
            {
                let model = self.get_current_model().await;
                Self::validate_tool_modality(&tool_input, model.as_ref())?;
            }

            self.call_with_timeout(&tool_name, || {
                self.tool_executor.execute(tool_input, context)
            })
            .await
        } else if self.agent_executor.contains_tool(&input.name).await? {
            // Handle agent delegation tool calls
            let agent_input = AgentInput::try_from(&input)?;
            let executor = self.agent_executor.clone();
            let agent_name = input.name.as_str().to_string();
            // NOTE: Agents should not timeout
            let outputs = join_all(agent_input.tasks.into_iter().map(|task| {
                let agent_name = agent_name.clone();
                let executor = executor.clone();
                async move {
                    executor
                        .execute(AgentId::new(&agent_name), task, context, None)
                        .await
                }
            }))
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(ToolOutput::from(outputs.into_iter()))
        } else if self.mcp_executor.contains_tool(&input.name).await? {
            let output = self
                .call_with_timeout(&tool_name, || self.mcp_executor.execute(input, context))
                .await?;
            let text = output
                .values
                .iter()
                .filter_map(|output| output.as_str())
                .fold(String::new(), |mut a, b| {
                    a.push('\n');
                    a.push_str(b);
                    a
                });
            if !text.trim().is_empty() {
                let text = style(text).cyan().dim().to_string();
                context
                    .send(ChatResponse::TaskMessage {
                        content: ChatResponseContent::ToolOutput(text),
                    })
                    .await?;
            }
            Ok(output)
        } else {
            Err(Error::NotFound(input.name).into())
        }
    }

    pub async fn call(
        &self,
        agent: &Agent,
        context: &ToolCallContext,
        call: ToolCallFull,
    ) -> ToolResult {
        let call_id = call.call_id.clone();
        let tool_name = call.name.clone();
        let output = self.call_inner(agent, call, context).await;

        ToolResult::new(tool_name).call_id(call_id).output(output)
    }

    pub async fn list(&self) -> anyhow::Result<Vec<ToolDefinition>> {
        Ok(self.tools_overview().await?.into())
    }

    /// Gets the model for the currently active agent by looking up the agent
    /// and fetching its model from the provider's model list.
    ///
    /// Returns None if no active agent, agent not found, or model not in
    /// provider list.
    async fn get_current_model(&self) -> Option<Model> {
        let agent_id = self.services.get_active_agent_id().await.ok()??;
        let agent = self.services.get_agent(&agent_id).await.ok()??;
        let provider = self.services.get_provider(agent.provider).await.ok()?;
        let models = self.services.models(provider).await.ok()?;
        models.iter().find(|m| m.id == agent.model).cloned()
    }

    pub async fn tools_overview(&self) -> anyhow::Result<ToolsOverview> {
        let mcp_tools = self.services.get_mcp_servers().await?;
        let agent_tools = self.agent_executor.agent_definitions().await?;

        // Get agents for template rendering in Task tool description
        let mut agents = self.services.get_agents().await?;

        // Check if current working directory is indexed
        let environment = self.services.get_environment();
        let cwd = environment.cwd.clone();
        let is_indexed = self.services.is_indexed(&cwd).await.unwrap_or(false);
        let is_authenticated = self.services.is_authenticated().await.unwrap_or(false);

        // Get current model for dynamic tool descriptions
        let model = self.get_current_model().await;

        // Build TemplateConfig from ForgeConfig for tool description templates
        let config = self.services.get_config()?;

        // Filter out research subagents from task tool description when disabled
        if !config.research_subagent {
            agents.retain(|agent| {
                let id = agent.id.as_str();
                id != "sage" && id != "agent"
            });
        }

        let template_config = TemplateConfig {
            max_read_size: config.max_read_lines as usize,
            max_line_length: config.max_line_chars,
            max_image_size: config.max_image_size_bytes as usize,
            stdout_max_prefix_length: config.max_stdout_prefix_lines,
            stdout_max_suffix_length: config.max_stdout_suffix_lines,
            stdout_max_line_length: config.max_stdout_line_chars,
        };

        Ok(ToolsOverview::new()
            .system(Self::get_system_tools(
                is_indexed && is_authenticated,
                &environment,
                model,
                agents,
                &template_config,
            ))
            .agents(agent_tools)
            .mcp(mcp_tools))
    }
}

impl<S> ToolRegistry<S> {
    fn get_system_tools(
        sem_search_supported: bool,
        env: &Environment,
        model: Option<Model>,
        agents: Vec<forge_domain::Agent>,
        template_config: &TemplateConfig,
    ) -> Vec<ToolDefinition> {
        use crate::TemplateEngine;

        let handlebars = TemplateEngine::handlebar_instance();
        let mut agents = agents;
        agents.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));

        // Build tool_names map from all available tools
        let tool_names: Map<String, Value> = ToolCatalog::iter()
            .filter(|tool| {
                // Only include tools that are supported (filter sem_search if not supported)
                if matches!(tool, ToolCatalog::SemSearch(_)) {
                    sem_search_supported
                } else {
                    true
                }
            })
            .map(|tool| {
                let def = tool.definition();
                (def.name.to_string(), json!(def.name.to_string()))
            })
            .collect();

        // Create template data with environment nested under "env"
        let ctx = SystemContext {
            env: Some(env.clone()),
            model,
            tool_names,
            agents,
            config: Some(template_config.clone()),
            ..Default::default()
        };

        ToolCatalog::iter()
            .filter(|tool| {
                // Filter out sem_search if cwd is not indexed
                if matches!(tool, ToolCatalog::SemSearch(_)) {
                    sem_search_supported
                } else {
                    true
                }
            })
            .map(|tool| {
                let mut def = tool.definition();
                // Render template variables in description
                if let Ok(rendered) = handlebars.render_template(&def.description, &ctx) {
                    def.description = rendered;
                }
                def
            })
            .collect::<Vec<_>>()
    }

    /// Validates if a tool is supported by both the agent and the system.
    ///
    /// # Validation Process
    /// Verifies the tool is supported by the agent specified in the context
    fn validate_tool_call(agent: &Agent, tool_name: &ToolName) -> Result<(), Error> {
        // Check if tool matches any pattern (supports globs like "mcp_*")
        let matches = ToolResolver::is_allowed(agent, tool_name);
        if !matches {
            tracing::error!(tool_name = %tool_name, "No tool with name");
            let supported_tools = agent
                .tools
                .iter()
                .flatten()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::NotAllowed { name: tool_name.clone(), supported_tools });
        }
        Ok(())
    }

    /// Checks if a file path has an image extension.
    /// This is a lightweight check that doesn't require reading the file.
    fn has_image_extension(path: &str) -> bool {
        const IMAGE_EXTENSIONS: &[&str] = &[
            ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".svg", ".pdf",
        ];

        let path_lower = path.to_lowercase();
        IMAGE_EXTENSIONS.iter().any(|ext| path_lower.ends_with(ext))
    }

    /// Validates if a tool's modality requirements are supported by the current
    /// model.
    ///
    /// # Validation Process
    /// Checks if the tool requires image input support and if the model
    /// supports it. Currently, only the `read` tool can potentially require
    /// image modality.
    fn validate_tool_modality(
        tool_input: &ToolCatalog,
        model: Option<&Model>,
    ) -> Result<(), Error> {
        // Check if this tool might require image support
        // Currently, only the read tool can return image content
        if let ToolCatalog::Read(input) = tool_input {
            // Check if the file extension suggests it's an image
            if Self::has_image_extension(&input.file_path) {
                // Check if the model supports image input
                let supports_image = model
                    .and_then(|m| {
                        m.input_modalities
                            .iter()
                            .find(|im| matches!(im, InputModality::Image))
                    })
                    .is_some();

                if !supports_image {
                    let tool_name = ToolKind::Read.name();
                    let required_modality = "image".to_string();
                    let supported_modalities = model
                        .map(|m| {
                            m.input_modalities
                                .iter()
                                .map(|im| match im {
                                    InputModality::Text => "text".to_string(),
                                    InputModality::Image => "image".to_string(),
                                })
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_else(|| "unknown".to_string());

                    return Err(Error::UnsupportedModality {
                        tool_name,
                        required_modality,
                        supported_modalities,
                    });
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{
        Agent, AgentId, Environment, ModelId, ProviderId, TemplateConfig, ToolCatalog, ToolName,
    };
    use pretty_assertions::assert_eq;

    use crate::error::Error;
    use crate::tool_registry::{ToolRegistry, create_test_agents};

    fn agent() -> Agent {
        // only allow read and search tools for this agent
        Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![ToolName::new("read"), ToolName::new("fs_search")])
    }

    #[tokio::test]
    async fn test_restricted_tool_call() {
        let result = ToolRegistry::<()>::validate_tool_call(
            &agent(),
            &ToolName::new(ToolCatalog::Read(Default::default())),
        );
        assert!(result.is_ok(), "Tool call should be valid");
    }

    #[tokio::test]
    async fn test_restricted_tool_call_err() {
        let error = ToolRegistry::<()>::validate_tool_call(&agent(), &ToolName::new("write"))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "Tool 'write' is not available. Please try again with one of these tools: [read, fs_search]"
        );
    }

    #[test]
    fn test_validate_tool_call_with_glob_pattern_wildcard() {
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![ToolName::new("mcp_*"), ToolName::new("read")]);

        let actual = ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("mcp_foo"));

        assert!(actual.is_ok());
    }

    #[test]
    fn test_validate_tool_call_with_glob_pattern_multiple_tools() {
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![ToolName::new("mcp_*"), ToolName::new("read")]);

        let actual_mcp_read =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("mcp_read"));
        let actual_mcp_write =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("mcp_write"));
        let actual_read = ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("read"));

        assert!(actual_mcp_read.is_ok());
        assert!(actual_mcp_write.is_ok());
        assert!(actual_read.is_ok());
    }

    #[test]
    fn test_validate_tool_call_with_glob_pattern_no_match() {
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![ToolName::new("mcp_*"), ToolName::new("read")]);

        let actual = ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("write"));

        let expected = Error::NotAllowed {
            name: ToolName::new("write"),
            supported_tools: "mcp_*, read".to_string(),
        }
        .to_string();

        assert_eq!(actual.unwrap_err().to_string(), expected);
    }

    #[test]
    fn test_validate_tool_call_with_glob_pattern_question_mark() {
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![ToolName::new("read?"), ToolName::new("write")]);

        let actual_read1 =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("read1"));
        let actual_readx =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("readx"));
        let actual_read = ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("read"));

        assert!(actual_read1.is_ok());
        assert!(actual_readx.is_ok());
        assert!(actual_read.is_err());
    }

    #[test]
    fn test_validate_tool_call_with_glob_pattern_character_class() {
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![ToolName::new("tool_[abc]"), ToolName::new("write")]);

        let actual_tool_a =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("tool_a"));
        let actual_tool_b =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("tool_b"));
        let actual_tool_c =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("tool_c"));
        let actual_tool_d =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("tool_d"));

        assert!(actual_tool_a.is_ok());
        assert!(actual_tool_b.is_ok());
        assert!(actual_tool_c.is_ok());
        assert!(actual_tool_d.is_err());
    }

    #[test]
    fn test_validate_tool_call_with_glob_pattern_double_wildcard() {
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![ToolName::new("**"), ToolName::new("read")]);

        let actual_any_tool =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("any_tool_name"));
        let actual_nested =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("nested/tool"));

        assert!(actual_any_tool.is_ok());
        assert!(actual_nested.is_ok());
    }

    #[test]
    fn test_validate_tool_call_exact_match_with_special_chars() {
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![ToolName::new("tool_[special]"), ToolName::new("read")]);

        let actual =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("tool_[special]"));

        // The glob pattern "tool_[special]" will match "tool_s", "tool_p", etc., not
        // the literal string So this test verifies that exact matching doesn't
        // work when the pattern is a valid glob
        assert!(actual.is_err());
    }

    #[test]
    fn test_validate_tool_call_backward_compatibility_exact_match() {
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![
            ToolName::new("read"),
            ToolName::new("write"),
            ToolName::new("fs_search"),
        ]);

        let actual_read = ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("read"));
        let actual_write =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("write"));
        let actual_invalid =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("delete"));

        assert!(actual_read.is_ok());
        assert!(actual_write.is_ok());
        assert!(actual_invalid.is_err());
    }

    #[test]
    fn test_validate_tool_call_empty_tools_list() {
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        );

        let actual = ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("read"));

        assert!(actual.is_err());
    }

    #[test]
    fn test_validate_tool_call_glob_with_prefix_suffix() {
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![ToolName::new("mcp_*_tool")]);

        let actual_match =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("mcp_read_tool"));
        let actual_no_match =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("mcp_read"));

        assert!(actual_match.is_ok());
        assert!(actual_no_match.is_err());
    }

    #[test]
    fn test_validate_tool_call_capitalized_read_write() {
        // Test that capitalized "Read" and "Write" are accepted when agent has
        // lowercase versions
        let fixture = Agent::new(
            AgentId::new("test_agent"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .tools(vec![ToolName::new("read"), ToolName::new("write")]);

        let actual_read = ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("Read"));
        let actual_write =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("Write"));
        let actual_lowercase_read =
            ToolRegistry::<()>::validate_tool_call(&fixture, &ToolName::new("read"));

        assert!(actual_read.is_ok(), "Capitalized 'Read' should be accepted");
        assert!(
            actual_write.is_ok(),
            "Capitalized 'Write' should be accepted"
        );
        assert!(
            actual_lowercase_read.is_ok(),
            "Lowercase 'read' should still be accepted"
        );
    }

    #[test]
    fn test_sem_search_included_when_supported() {
        use fake::{Fake, Faker};
        let env: Environment = Faker.fake();
        let template_config = TemplateConfig::default();
        let actual = ToolRegistry::<()>::get_system_tools(
            true,
            &env,
            None,
            create_test_agents(),
            &template_config,
        );
        assert!(actual.iter().any(|t| t.name.as_str() == "sem_search"));
    }

    #[test]
    fn test_sem_search_filtered_when_not_supported() {
        use fake::{Fake, Faker};
        let env: Environment = Faker.fake();
        let template_config = TemplateConfig::default();
        let actual = ToolRegistry::<()>::get_system_tools(
            false,
            &env,
            None,
            create_test_agents(),
            &template_config,
        );
        assert!(actual.iter().all(|t| t.name.as_str() != "sem_search"));
    }

    #[test]
    fn test_task_tool_description_is_stable_across_agent_order() {
        use fake::{Fake, Faker};
        let env: Environment = Faker.fake();
        let template_config = TemplateConfig::default();
        let agents = create_test_agents();
        let mut reversed_agents = agents.clone();
        reversed_agents.reverse();

        let fixture =
            ToolRegistry::<()>::get_system_tools(true, &env, None, agents, &template_config);
        let actual = ToolRegistry::<()>::get_system_tools(
            true,
            &env,
            None,
            reversed_agents,
            &template_config,
        );

        let expected = fixture
            .iter()
            .find(|tool| tool.name.as_str() == "task")
            .expect("Task tool should exist")
            .description
            .clone();
        let actual = actual
            .iter()
            .find(|tool| tool.name.as_str() == "task")
            .expect("Task tool should exist")
            .description
            .clone();

        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
fn create_test_agents() -> Vec<forge_domain::Agent> {
    use forge_domain::{Agent, AgentId, ModelId, ProviderId, ToolName};

    vec![
        Agent::new(
            AgentId::new("sage"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .id(AgentId::new("sage"))
        .title("Research Agent")
        .description("Specialized in researching codebases")
        .tools(vec![
            ToolName::new("read"),
            ToolName::new("fs_search"),
            ToolName::new("sem_search"),
            ToolName::new("fetch"),
        ]),
        Agent::new(
            AgentId::new("debug"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-3-5-sonnet-20241022"),
        )
        .id(AgentId::new("debug"))
        .title("Debug Agent")
        .description("Specialized in debugging issues")
        .tools(vec![
            ToolName::new("read"),
            ToolName::new("shell"),
            ToolName::new("fs_search"),
            ToolName::new("sem_search"),
            ToolName::new("fetch"),
        ]),
    ]
}

#[cfg(test)]
fn create_test_model(
    id: &str,
    modalities: Vec<forge_domain::InputModality>,
) -> forge_domain::Model {
    use forge_domain::{Model, ModelId};

    Model {
        id: ModelId::new(id),
        name: Some(format!("Test {}", id)),
        description: None,
        context_length: Some(128000),
        tools_supported: Some(true),
        supports_parallel_tool_calls: Some(true),
        supports_reasoning: Some(false),
        input_modalities: modalities,
    }
}

#[test]
fn test_template_rendering_in_tool_descriptions() {
    use fake::{Fake, Faker};

    let env: Environment = Faker.fake();
    let template_config = TemplateConfig { max_line_length: 2000, ..Default::default() };

    let actual = ToolRegistry::<()>::get_system_tools(
        true,
        &env,
        None,
        create_test_agents(),
        &template_config,
    );
    let fs_search_tool = actual
        .iter()
        .find(|t| t.name.as_str() == "fs_search")
        .unwrap();

    // The description should not contain unrendered template variables
    assert!(
        !fs_search_tool.description.contains("{{"),
        "Description should not contain unrendered template variable: {}",
        fs_search_tool.description
    );

    // The description should contain the expected usage info
    assert!(
        fs_search_tool.description.contains("ripgrep"),
        "Description should mention ripgrep: {}",
        fs_search_tool.description
    );
}

#[test]
fn test_dynamic_tool_description_with_vision_model() {
    use fake::{Fake, Faker};
    use forge_domain::InputModality;

    let env: Environment = Faker.fake();
    let template_config = TemplateConfig {
        max_read_size: 2000,
        max_line_length: 2000,
        max_image_size: 5000,
        ..Default::default()
    };
    let vision_model = create_test_model("gpt-4o", vec![InputModality::Text, InputModality::Image]);

    let tools_with_vision = ToolRegistry::<()>::get_system_tools(
        true,
        &env,
        Some(vision_model),
        create_test_agents(),
        &template_config,
    );
    let read_tool = tools_with_vision
        .iter()
        .find(|t| t.name.as_str() == "read")
        .unwrap();
    insta::assert_snapshot!(read_tool.description);
}

#[test]
fn test_dynamic_tool_description_with_text_only_model() {
    use fake::{Fake, Faker};
    use forge_domain::InputModality;

    let env: Environment = Faker.fake();
    let template_config = TemplateConfig {
        max_read_size: 2000,
        max_line_length: 2000,
        max_image_size: 5000,
        ..Default::default()
    };
    let text_only_model = create_test_model("gpt-3.5-turbo", vec![InputModality::Text]);

    let tools_text_only = ToolRegistry::<()>::get_system_tools(
        true,
        &env,
        Some(text_only_model),
        create_test_agents(),
        &template_config,
    );
    let read_tool = tools_text_only
        .iter()
        .find(|t| t.name.as_str() == "read")
        .unwrap();

    // Text-only model should NOT see image and PDF support
    insta::assert_snapshot!(read_tool.description);
}

#[test]
fn test_validate_tool_modality_with_image_file_and_vision_model() {
    use forge_domain::{InputModality, ToolCatalog};

    let vision_model = create_test_model("gpt-4o", vec![InputModality::Text, InputModality::Image]);
    let tool_input = ToolCatalog::Read(forge_domain::FSRead {
        file_path: "/home/user/test.png".to_string(),
        ..Default::default()
    });

    let result = ToolRegistry::<()>::validate_tool_modality(&tool_input, Some(&vision_model));
    assert!(result.is_ok(), "Vision model should support image files");
}

#[test]
fn test_validate_tool_modality_with_image_file_and_text_only_model() {
    use forge_domain::{InputModality, ToolCatalog};

    let text_only_model = create_test_model("gpt-3.5-turbo", vec![InputModality::Text]);
    let tool_input = ToolCatalog::Read(forge_domain::FSRead {
        file_path: "/home/user/test.png".to_string(),
        ..Default::default()
    });

    let result = ToolRegistry::<()>::validate_tool_modality(&tool_input, Some(&text_only_model));
    assert!(
        result.is_err(),
        "Text-only model should not support image files"
    );

    let error = result.unwrap_err();
    assert!(error.to_string().contains("requires image modality"));
    assert!(error.to_string().contains("read"));
}

#[test]
fn test_validate_tool_modality_with_text_file_and_text_only_model() {
    use forge_domain::{InputModality, ToolCatalog};

    let text_only_model = create_test_model("gpt-3.5-turbo", vec![InputModality::Text]);
    let tool_input = ToolCatalog::Read(forge_domain::FSRead {
        file_path: "/home/user/test.txt".to_string(),
        ..Default::default()
    });

    let result = ToolRegistry::<()>::validate_tool_modality(&tool_input, Some(&text_only_model));
    assert!(result.is_ok(), "Text-only model should support text files");
}

#[test]
fn test_validate_tool_modality_with_no_model() {
    use forge_domain::ToolCatalog;

    let tool_input = ToolCatalog::Read(forge_domain::FSRead {
        file_path: "/home/user/test.png".to_string(),
        ..Default::default()
    });

    let result = ToolRegistry::<()>::validate_tool_modality(&tool_input, None);
    assert!(result.is_err(), "Should error when no model is available");

    let error = result.unwrap_err();
    assert!(error.to_string().contains("requires image modality"));
    assert!(error.to_string().contains("unknown"));
}

#[test]
fn test_validate_tool_modality_with_non_read_tool() {
    use forge_domain::{InputModality, ToolCatalog};

    let text_only_model = create_test_model("gpt-3.5-turbo", vec![InputModality::Text]);
    let tool_input = ToolCatalog::Write(forge_domain::FSWrite {
        file_path: "/home/user/test.png".to_string(),
        content: "test".to_string(),
        ..Default::default()
    });

    let result = ToolRegistry::<()>::validate_tool_modality(&tool_input, Some(&text_only_model));
    assert!(
        result.is_ok(),
        "Non-read tools should pass modality validation"
    );
}

#[test]
fn test_has_image_extension() {
    // Test various image extensions (case-insensitive)
    assert!(ToolRegistry::<()>::has_image_extension("/path/to/file.png"));
    assert!(ToolRegistry::<()>::has_image_extension("/path/to/file.PNG"));
    assert!(ToolRegistry::<()>::has_image_extension("/path/to/file.jpg"));
    assert!(ToolRegistry::<()>::has_image_extension(
        "/path/to/file.jpeg"
    ));
    assert!(ToolRegistry::<()>::has_image_extension(
        "/path/to/file.JPEG"
    ));
    assert!(ToolRegistry::<()>::has_image_extension("/path/to/file.gif"));
    assert!(ToolRegistry::<()>::has_image_extension("/path/to/file.bmp"));
    assert!(ToolRegistry::<()>::has_image_extension(
        "/path/to/file.webp"
    ));
    assert!(ToolRegistry::<()>::has_image_extension("/path/to/file.svg"));

    // Test relative paths
    assert!(ToolRegistry::<()>::has_image_extension("image.png"));
    assert!(ToolRegistry::<()>::has_image_extension(
        "../images/photo.jpg"
    ));
    assert!(ToolRegistry::<()>::has_image_extension("/path/to/file.pdf"));

    // Test non-image files
    assert!(!ToolRegistry::<()>::has_image_extension(
        "/path/to/file.txt"
    ));
    assert!(!ToolRegistry::<()>::has_image_extension("/path/to/file.rs"));
    assert!(!ToolRegistry::<()>::has_image_extension("/path/to/file"));
    assert!(!ToolRegistry::<()>::has_image_extension("README.md"));

    // Test edge cases
    assert!(!ToolRegistry::<()>::has_image_extension(""));
    assert!(ToolRegistry::<()>::has_image_extension(
        "file.with.dots.png"
    ));
    assert!(ToolRegistry::<()>::has_image_extension(".png")); // Hidden file with .png extension
}

#[test]
fn test_dynamic_tool_description_without_model() {
    use fake::{Fake, Faker};

    let env: Environment = Faker.fake();
    let template_config = TemplateConfig {
        max_read_size: 2000,
        max_image_size: 5000,
        max_line_length: 2000,
        ..Default::default()
    };

    // When no model is provided, should default to showing minimal capabilities
    let tools_no_model = ToolRegistry::<()>::get_system_tools(
        true,
        &env,
        None,
        create_test_agents(),
        &template_config,
    );
    let read_tool = tools_no_model
        .iter()
        .find(|t| t.name.as_str() == "read")
        .unwrap();

    // Without model info, should show basic text file support
    insta::assert_snapshot!(read_tool.description);
}

#[test]
fn test_all_rendered_tool_descriptions() {
    use fake::{Fake, Faker};

    let mut env: Environment = Faker.fake();
    env.cwd = "/home/user/project".into();

    let template_config = TemplateConfig {
        max_read_size: 2000,
        max_line_length: 2000,
        max_image_size: 5000,
        stdout_max_prefix_length: 200,
        stdout_max_suffix_length: 200,
        stdout_max_line_length: 2000,
    };

    let tools = ToolRegistry::<()>::get_system_tools(
        true,
        &env,
        None,
        create_test_agents(),
        &template_config,
    );

    // Verify all tools have rendered descriptions (no template syntax left)
    for tool in &tools {
        assert!(
            !tool.description.contains("{{"),
            "Tool '{}' has unrendered template variables:\n{}",
            tool.name,
            tool.description
        );
    }

    // Snapshot all rendered tool descriptions for visual verification
    // This will fail if a tool is renamed and descriptions reference the old name
    let all_descriptions: Vec<_> = tools
        .iter()
        .map(|t| format!("### {}\n\n{}\n", t.name, t.description))
        .collect();

    insta::assert_snapshot!(
        "all_rendered_tool_descriptions",
        all_descriptions.join("\n---\n\n")
    );
}
