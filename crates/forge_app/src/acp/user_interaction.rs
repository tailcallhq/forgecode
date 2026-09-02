//! User prompts over ACP.
//!
//! Forge asks the user questions from inside tool execution: policy
//! confirmations, MCP trust prompts and follow-up choices. On a terminal those
//! are interactive pickers. When forge runs as an ACP agent its stdin is the
//! JSON-RPC pipe, so the same questions are forwarded to the client as
//! `session/request_permission` and answered from the client's UI.
//!
//! Tool execution runs on `Send` futures while the ACP connection is
//! single-threaded, so requests cross over a channel and are answered by the
//! connection task in `AcpApp::start_stdio`.
use tokio::sync::{mpsc, oneshot};

use crate::UserInfra;

/// One question with a closed set of answers. `reply` carries the index of
/// the chosen option, or `None` when the client cancelled or rejected.
pub struct UserChoiceRequest {
    pub message: String,
    pub options: Vec<String>,
    pub reply: oneshot::Sender<Option<usize>>,
}

pub type UserChoiceReceiver = mpsc::Receiver<UserChoiceRequest>;

/// `UserInfra` transport that forwards questions to the ACP client.
#[derive(Clone)]
pub struct AcpUserInteraction {
    tx: mpsc::Sender<UserChoiceRequest>,
}

/// Creates the sending half (installed in the infra as the `UserInfra`
/// transport) and the receiving half (drained by the ACP connection task).
pub fn acp_user_interaction() -> (AcpUserInteraction, UserChoiceReceiver) {
    let (tx, rx) = mpsc::channel(8);
    (AcpUserInteraction { tx }, rx)
}

impl AcpUserInteraction {
    async fn select_index(&self, message: &str, options: Vec<String>) -> anyhow::Result<Option<usize>> {
        let (reply, answer) = oneshot::channel();
        tracing::debug!(message, ?options, "Forwarding user question to ACP client");
        self.tx
            .send(UserChoiceRequest { message: message.to_string(), options, reply })
            .await
            .map_err(|_| anyhow::anyhow!("ACP connection is closed; cannot ask the user"))?;
        answer
            .await
            .map_err(|_| anyhow::anyhow!("ACP connection dropped the user prompt"))
    }
}

#[async_trait::async_trait]
impl UserInfra for AcpUserInteraction {
    async fn prompt_question(&self, question: &str) -> anyhow::Result<Option<String>> {
        anyhow::bail!("free-text questions are not supported over ACP: {question}")
    }

    async fn select_one<T: Clone + std::fmt::Display + Send + 'static>(
        &self,
        message: &str,
        mut options: Vec<T>,
    ) -> anyhow::Result<Option<T>> {
        if options.is_empty() {
            return Ok(None);
        }
        let labels = options.iter().map(ToString::to_string).collect();
        Ok(self
            .select_index(message, labels)
            .await?
            .filter(|index| *index < options.len())
            .map(|index| options.swap_remove(index)))
    }

    async fn select_many<T: std::fmt::Display + Clone + Send + 'static>(
        &self,
        message: &str,
        _options: Vec<T>,
    ) -> anyhow::Result<Option<Vec<T>>> {
        anyhow::bail!("multi-select questions are not supported over ACP: {message}")
    }
}
