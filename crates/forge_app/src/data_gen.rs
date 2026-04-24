use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use forge_domain::{
    Context, ContextMessage, DataGenerationParameters, ResultStreamExt, Template, ToolDefinition,
};
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use schemars::Schema;
use tracing::{debug, info};

use crate::{AppConfigService, FsReadService, ProviderService, Services, TemplateEngine};

pub struct DataGenerationApp<A> {
    services: Arc<A>,
}

type JsonSchema = String;
type SystemPrompt = String;
type UserPrompt = String;
type Input = Vec<serde_json::Value>;

impl<A: Services> DataGenerationApp<A> {
    pub fn new(services: Arc<A>) -> Self {
        Self { services }
    }

    /// Helper function to read a file from a path, resolving it relative to cwd
    /// if necessary
    async fn read_file(&self, path: PathBuf) -> Result<String> {
        let resolved_path = if path.is_absolute() {
            path
        } else {
            let cwd = self.services.get_environment().cwd;
            cwd.join(path)
        };

        let content = self
            .services
            .read(resolved_path.display().to_string(), None, None)
            .await?
            .content
            .file_content()
            .to_owned();

        Ok(content)
    }

    async fn read_file_opt(&self, path: Option<PathBuf>) -> Result<Option<String>> {
        match path {
            Some(path) => self.read_file(path).await.map(Some),
            None => Ok(None),
        }
    }

    async fn load_parameters(
        &self,
        params: DataGenerationParameters,
    ) -> Result<(JsonSchema, Option<SystemPrompt>, Option<UserPrompt>, Input)> {
        debug!("Loading data generation parameters");

        // Read all files in parallel
        let (schema, system_prompt, user_prompt, input) = tokio::join!(
            self.read_file(params.schema.clone()),
            self.read_file_opt(params.system_prompt),
            self.read_file_opt(params.user_prompt),
            self.read_file(params.input)
        );

        let input: Vec<serde_json::Value> = input?
            .lines()
            .map(|text| {
                serde_json::from_str(text).with_context(|| "Could not parse the input file")
            })
            .collect::<Result<Vec<_>>>()?;

        debug!("Loaded {} input items", input.len());

        Ok((schema?, system_prompt?, user_prompt?, input))
    }

    pub async fn execute(
        &self,
        params: DataGenerationParameters,
    ) -> Result<BoxStream<'static, Result<serde_json::Value>>> {
        let concurrency = params.concurrency;
        let (schema, system_prompt, user_prompt, input) = self.load_parameters(params).await?;

        info!(
            "Starting data generation with {} items (concurrency: {})",
            input.len(),
            concurrency
        );

        let model_config = self
            .services
            .get_session_config()
            .await
            .ok_or_else(|| forge_domain::Error::NoDefaultSession)?;
        let provider = self.services.get_provider(model_config.provider).await?;
        let model_id = model_config.model;
        debug!("Using provider: {}, model: {}", provider.id, model_id);
        let schema: Schema =
            serde_json::from_str(&schema).with_context(|| "Could not parse the JSON schema")?;
        let mut context =
            Context::default().add_tool(ToolDefinition::new("output").input_schema(schema));

        if let Some(content) = system_prompt {
            context = context.add_message(ContextMessage::system(content))
        }

        let services = self.services.clone();

        let json_stream = input.into_iter().map(move |input| {
            let provider = provider.clone();
            let context = context.clone();
            let user_prompt = user_prompt.clone();
            let model_id = model_id.clone();
            let services = services.clone();

            async move {
                debug!("Processing data generation request");

                let provider = provider.clone();
                let mut context = context.clone();
                let content = if let Some(ref content) = user_prompt {
                    TemplateEngine::default().render_template(Template::new(content), &input)?
                } else {
                    serde_json::to_string(&input)?
                };

                context =
                    context.add_message(ContextMessage::user(content, Some(model_id.clone())));

                let stream = services.chat(&model_id, context, provider.clone()).await?;
                let response = stream.into_full(false).await?;

                anyhow::Ok((input, response))
            }
        });

        let json_stream = stream::iter(json_stream)
            .buffer_unordered(concurrency)
            .map(|result| {
                result.and_then(|(input, response)| {
                    response
                        .tool_calls
                        .into_iter()
                        .map(|tool| {
                            let output = tool.arguments.parse()?;
                            let mut value = serde_json::Map::new();
                            value.insert("input".to_string(), input.clone());
                            value.insert("output".to_string(), output);
                            Ok(serde_json::Value::from(value))
                        })
                        .collect::<Result<Vec<_>>>()
                })
            })
            .flat_map(|data| match data {
                Ok(data) => stream::iter(data).map(Ok).boxed(),
                Err(err) => stream::iter(Err(err)).boxed(),
            })
            .boxed();

        Ok(json_stream)
    }
}
