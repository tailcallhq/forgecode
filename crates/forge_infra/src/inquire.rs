use anyhow::Result;
use forge_app::UserInfra;
use forge_select::ForgeWidget;

/// Where user questions go: an interactive terminal picker, or the ACP client
/// when forge runs as an ACP agent and stdin is the protocol pipe.
pub enum ForgeInquire {
    Terminal,
    Acp(forge_app::AcpUserInteraction),
}

impl Default for ForgeInquire {
    fn default() -> Self {
        Self::new()
    }
}

impl ForgeInquire {
    pub fn new() -> Self {
        Self::Terminal
    }

    async fn prompt<T, F>(&self, f: F) -> Result<Option<T>>
    where
        F: FnOnce() -> Result<Option<T>> + Send + 'static,
        T: Send + 'static,
    {
        tokio::task::spawn_blocking(f).await?
    }
}

#[async_trait::async_trait]
impl UserInfra for ForgeInquire {
    async fn prompt_question(&self, question: &str) -> Result<Option<String>> {
        if let Self::Acp(acp) = self {
            return acp.prompt_question(question).await;
        }
        let question = question.to_string();
        self.prompt(move || ForgeWidget::input(&question).allow_empty(true).prompt())
            .await
    }

    async fn select_one<T: Clone + std::fmt::Display + Send + 'static>(
        &self,
        message: &str,
        options: Vec<T>,
    ) -> Result<Option<T>> {
        if let Self::Acp(acp) = self {
            return acp.select_one(message, options).await;
        }
        if options.is_empty() {
            return Ok(None);
        }

        let message = message.to_string();
        self.prompt(move || ForgeWidget::select(&message, options).prompt())
            .await
    }

    async fn select_many<T: std::fmt::Display + Clone + Send + 'static>(
        &self,
        message: &str,
        options: Vec<T>,
    ) -> Result<Option<Vec<T>>> {
        if let Self::Acp(acp) = self {
            return acp.select_many(message, options).await;
        }
        if options.is_empty() {
            return Ok(None);
        }

        let message = message.to_string();
        self.prompt(move || ForgeWidget::multi_select(&message, options).prompt())
            .await
    }
}
