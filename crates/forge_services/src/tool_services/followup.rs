use std::sync::Arc;

use forge_app::{FollowUpService, UserInfra};

/// Use this tool when you encounter ambiguities, need clarification, or require
/// more details to proceed effectively. Use this tool judiciously to maintain a
/// balance between gathering necessary information and avoiding excessive
/// back-and-forth.
#[derive(Debug)]
pub struct ForgeFollowup<F> {
    infra: Arc<F>,
}

impl<F> ForgeFollowup<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<F: UserInfra> FollowUpService for ForgeFollowup<F> {
    async fn follow_up(
        &self,
        question: String,
        options: Vec<String>,
        multiple: Option<bool>,
    ) -> anyhow::Result<Option<String>> {
        let inquire = &self.infra;
        let result = match (options.is_empty(), multiple.unwrap_or_default()) {
            (true, _) => inquire.prompt_question(&question).await?,
            (false, true) => inquire
                .select_many(&question, options)
                .await?
                .map(|selected| {
                    format!(
                        "User selected {} option(s): {}",
                        selected.len(),
                        selected.join(", ")
                    )
                }),
            (false, false) => inquire
                .select_one(question.as_str(), options)
                .await?
                .map(|selected| format!("User selected: {selected}")),
        };

        Ok(result)
    }
}
