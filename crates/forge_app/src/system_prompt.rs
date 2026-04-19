use std::collections::HashMap;
use std::sync::Arc;

use derive_setters::Setters;
use forge_domain::{
    Agent, Conversation, Environment, Extension, ExtensionStat, File, Model, SystemContext,
    Template, TemplateConfig, ToolCatalog, ToolDefinition, ToolUsagePrompt,
};
use serde_json::{Map, Value, json};
use strum::IntoEnumIterator;
use tracing::debug;

use crate::{ShellService, SkillFetchService, TemplateEngine};

#[derive(Setters)]
pub struct SystemPrompt<S> {
    services: Arc<S>,
    environment: Environment,
    agent: Agent,
    tool_definitions: Vec<ToolDefinition>,
    files: Vec<File>,
    models: Vec<Model>,
    custom_instructions: Vec<String>,
    /// Maximum number of file extensions shown in the workspace summary.
    max_extensions: usize,
    /// Configuration values passed into tool description templates.
    template_config: TemplateConfig,
}

impl<S: SkillFetchService + ShellService> SystemPrompt<S> {
    pub fn new(services: Arc<S>, environment: Environment, agent: Agent) -> Self {
        Self {
            services,
            environment,
            agent,
            models: Vec::default(),
            tool_definitions: Vec::default(),
            files: Vec::default(),
            custom_instructions: Vec::default(),
            max_extensions: 0,
            template_config: TemplateConfig::default(),
        }
    }

    /// Fetches file extension statistics by running git ls-files command.
    async fn fetch_extensions(&self, max_extensions: usize) -> Option<Extension> {
        let output = self
            .services
            .execute(
                "git ls-files".into(),
                self.environment.cwd.clone(),
                false,
                true,
                None,
                None,
            )
            .await
            .ok()?;

        // If git command fails (e.g., not in a git repo), return None
        if output.output.exit_code != Some(0) {
            return None;
        }

        parse_extensions(&output.output.stdout, max_extensions)
    }

    pub async fn add_system_message(
        &self,
        mut conversation: Conversation,
    ) -> anyhow::Result<Conversation> {
        let context = conversation.context.take().unwrap_or_default();
        let agent = &self.agent;
        let context = if let Some(system_prompt) = &agent.system_prompt {
            let env = self.environment.clone();
            let files = self.files.clone();

            let tool_supported = self.is_tool_supported()?;
            let supports_parallel_tool_calls = self.is_parallel_tool_call_supported();
            let tool_information = match tool_supported {
                true => None,
                false => Some(ToolUsagePrompt::from(&self.tool_definitions).to_string()),
            };

            let mut custom_rules = Vec::new();

            agent.custom_rules.iter().for_each(|rule| {
                custom_rules.push(rule.as_str());
            });

            self.custom_instructions.iter().for_each(|rule| {
                custom_rules.push(rule.as_str());
            });

            let skills = self.services.list_skills().await?;

            // Fetch extension statistics from git
            let extensions = self.fetch_extensions(self.max_extensions).await;

            // Build tool_names map filtered to only the tools this agent actually has.
            // This allows templates to use {{#if tool_names.task}} to conditionally
            // render content based on whether the agent has access to a given tool.
            let agent_tool_names: std::collections::HashSet<String> = self
                .tool_definitions
                .iter()
                .map(|def| def.name.to_string())
                .collect();
            let tool_names: Map<String, Value> = ToolCatalog::iter()
                .map(|tool| {
                    let def = tool.definition();
                    (def.name.to_string(), json!(def.name.to_string()))
                })
                .filter(|(name, _)| agent_tool_names.contains(name))
                .collect();

            let ctx = SystemContext {
                env: Some(env),
                tool_information,
                tool_supported,
                files,
                custom_rules: custom_rules.join("\n\n"),
                supports_parallel_tool_calls,
                skills,
                model: None,
                tool_names,
                extensions,
                agents: vec![],
                config: None,
            };

            let static_block = TemplateEngine::default()
                .render_template(Template::new(&system_prompt.template), &ctx)?;
            let non_static_block = TemplateEngine::default()
                .render_template(Template::new("{{> forge-custom-agent-template.md }}"), &ctx)?;

            context.set_system_messages(vec![static_block, non_static_block])
        } else {
            context
        };

        Ok(conversation.context(context))
    }

    // Returns if agent supports tool or not.
    fn is_tool_supported(&self) -> anyhow::Result<bool> {
        let agent = &self.agent;
        let model_id = &agent.model;

        // Check if at agent level tool support is defined
        let tool_supported = match agent.tool_supported {
            Some(tool_supported) => tool_supported,
            None => {
                // If not defined at agent level, check model level

                let model = self.models.iter().find(|model| &model.id == model_id);
                model
                    .and_then(|model| model.tools_supported)
                    .unwrap_or_default()
            }
        };

        debug!(
            agent_id = %agent.id,
            model_id = %model_id,
            tool_supported,
            "Tool support check"
        );
        Ok(tool_supported)
    }

    /// Checks if parallel tool calls is supported by agent
    fn is_parallel_tool_call_supported(&self) -> bool {
        let agent = &self.agent;
        self.models
            .iter()
            .find(|model| model.id == agent.model)
            .and_then(|model| model.supports_parallel_tool_calls)
            .unwrap_or_default()
    }
}

/// Parses the newline-separated output of `git ls-files` into an [`Extension`]
/// summary.
fn parse_extensions(extensions: &str, max_extensions: usize) -> Option<Extension> {
    let all_files: Vec<&str> = extensions
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    let total_files = all_files.len();
    if total_files == 0 {
        return None;
    }

    // Count files by extension; files without extensions are tracked as "(no ext)"
    let mut counts = HashMap::<&str, usize>::new();
    all_files
        .iter()
        .map(|line| {
            let file_name = line.rsplit_once(['/', '\\']).map_or(*line, |(_, f)| f);
            file_name
                .rsplit_once('.')
                .filter(|(prefix, _)| !prefix.is_empty())
                .map_or("(no ext)", |(_, ext)| ext)
        })
        .for_each(|ext| *counts.entry(ext).or_default() += 1);

    // Convert to ExtensionStat and sort by count descending, then alphabetically
    let mut stats: Vec<_> = counts
        .into_iter()
        .map(|(extension, count)| {
            let percentage = ((count * 100) as f32 / total_files as f32).round() as usize;
            ExtensionStat {
                extension: extension.to_owned(),
                count,
                percentage: percentage.to_string(),
            }
        })
        .collect();

    stats.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.extension.cmp(&b.extension))
    });

    let total_extensions = stats.len();
    stats.truncate(max_extensions);

    // Calculate the count and percentage of files in remaining extensions after
    // truncation
    let shown_count: usize = stats.iter().map(|s| s.count).sum();
    let remaining_count = total_files.saturating_sub(shown_count);
    let remaining_percentage = ((remaining_count * 100) as f32 / total_files as f32)
        .ceil()
        .to_string();

    Some(Extension {
        extension_stats: stats,
        git_tracked_files: total_files,
        max_extensions,
        total_extensions,
        remaining_percentage,
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    const MAX_EXTENSIONS: usize = 15;

    #[test]
    fn test_parse_extensions_sorts_git_output() {
        let fixture = include_str!("fixtures/git_ls_files_mixed.txt");
        let actual = parse_extensions(fixture, MAX_EXTENSIONS).unwrap();

        // 9 files: 4 rs, 2 md, 2 no-ext, 1 toml — sorted by count desc then alpha
        let expected = Extension::new(
            vec![
                ExtensionStat::new("rs", 4, "44"),
                ExtensionStat::new("(no ext)", 2, "22"),
                ExtensionStat::new("md", 2, "22"),
                ExtensionStat::new("toml", 1, "11"),
            ],
            MAX_EXTENSIONS,
            9,
            4,
            "0",
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_extensions_truncates_to_max() {
        // Real `git ls-files` output from this repo: 822 files, 19 distinct extensions.
        // Top 15 are shown; the remaining 4 (html, jsonl, lock, proto — 1 each) are
        // rolled up.
        let fixture = include_str!("fixtures/git_ls_files_many_extensions.txt");
        let actual = parse_extensions(fixture, MAX_EXTENSIONS).unwrap();

        let expected = Extension::new(
            vec![
                ExtensionStat::new("rs", 415, "50"),
                ExtensionStat::new("snap", 159, "19"),
                ExtensionStat::new("md", 91, "11"),
                ExtensionStat::new("yml", 29, "4"),
                ExtensionStat::new("toml", 28, "3"),
                ExtensionStat::new("json", 22, "3"),
                ExtensionStat::new("zsh", 20, "2"),
                ExtensionStat::new("sql", 14, "2"),
                ExtensionStat::new("sh", 11, "1"),
                ExtensionStat::new("ts", 9, "1"),
                ExtensionStat::new("(no ext)", 7, "1"),
                ExtensionStat::new("txt", 5, "1"),
                ExtensionStat::new("csv", 4, "0"),
                ExtensionStat::new("yaml", 3, "0"),
                ExtensionStat::new("css", 1, "0"),
            ],
            MAX_EXTENSIONS,
            822,
            19,
            "1",
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_extensions_returns_none_for_empty_output() {
        assert_eq!(parse_extensions("", MAX_EXTENSIONS), None);
        assert_eq!(parse_extensions("   \n  \n", MAX_EXTENSIONS), None);
    }
}
