use std::collections::HashMap;
use std::sync::Arc;

use agent_client_protocol as acp;
use forge_domain::{AgentId, ConversationId, ModelId};
use tokio::sync::{Mutex, Notify, mpsc};

use super::error::{Error, Result};

/// Maximum number of buffered session notifications before backpressure.
const NOTIFICATION_CHANNEL_CAPACITY: usize = 1024;

#[derive(Clone)]
pub(super) struct SessionState {
    pub conversation_id: ConversationId,
    pub agent_id: AgentId,
    /// Session-scoped model override. When set, prompts use this model
    /// instead of the global default.
    pub model_id: Option<ModelId>,
    pub cancel_notify: Option<Arc<Notify>>,
}

pub(crate) struct AcpAdapter<S> {
    pub(super) services: Arc<S>,
    pub(super) session_update_tx: mpsc::Sender<acp::SessionNotification>,
    pub(super) client_conn: Arc<Mutex<Option<Arc<acp::AgentSideConnection>>>>,
    sessions: Arc<Mutex<HashMap<String, SessionState>>>,
}

impl<S> AcpAdapter<S> {
    fn with_services(services: Arc<S>) -> (Self, mpsc::Receiver<acp::SessionNotification>) {
        let (tx, rx) = mpsc::channel(NOTIFICATION_CHANNEL_CAPACITY);
        let adapter = Self {
            services,
            session_update_tx: tx,
            client_conn: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        };
        (adapter, rx)
    }

    #[cfg(test)]
    pub(super) fn new_for_test(services: S) -> Self {
        Self::with_services(Arc::new(services)).0
    }

    #[cfg(test)]
    pub(super) fn new_for_test_with_receiver(
        services: S,
    ) -> (Self, mpsc::Receiver<acp::SessionNotification>) {
        Self::with_services(Arc::new(services))
    }
}

impl<S> AcpAdapter<S> {
    pub(crate) async fn set_client_connection(&self, conn: Arc<acp::AgentSideConnection>) {
        *self.client_conn.lock().await = Some(conn);
    }

    pub(super) async fn store_session(&self, session_id: String, state: SessionState) {
        self.sessions.lock().await.insert(session_id, state);
    }

    /// Removes a session from the adapter. Currently unused but available
    /// for future session lifecycle management (TTL, explicit close).
    #[allow(dead_code)]
    pub(super) async fn remove_session(&self, session_id: &str) {
        self.sessions.lock().await.remove(session_id);
    }

    pub(super) async fn session_state(&self, session_id: &str) -> Result<SessionState> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| Error::Application(anyhow::anyhow!("Session not found")))
    }

    pub(super) async fn update_session_agent(
        &self,
        session_id: &str,
        agent_id: AgentId,
    ) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        let state = sessions
            .get_mut(session_id)
            .ok_or_else(|| Error::Application(anyhow::anyhow!("Session not found")))?;
        state.agent_id = agent_id;
        Ok(())
    }

    pub(super) async fn update_session_model(
        &self,
        session_id: &str,
        model_id: ModelId,
    ) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        let state = sessions
            .get_mut(session_id)
            .ok_or_else(|| Error::Application(anyhow::anyhow!("Session not found")))?;
        state.model_id = Some(model_id);
        Ok(())
    }

    pub(super) async fn set_cancel_notify(
        &self,
        session_id: &str,
        cancel_notify: Option<Arc<Notify>>,
    ) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        let state = sessions
            .get_mut(session_id)
            .ok_or_else(|| Error::Application(anyhow::anyhow!("Session not found")))?;
        state.cancel_notify = cancel_notify;
        Ok(())
    }

    pub(super) async fn cancel_session(&self, session_id: &str) -> bool {
        let notify = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .and_then(|state| state.cancel_notify.clone());

        if let Some(notify) = notify {
            notify.notify_waiters();
            true
        } else {
            false
        }
    }

    pub(super) async fn ensure_session(
        &self,
        session_id: &str,
        conversation_id: ConversationId,
        agent_id: AgentId,
    ) -> SessionState {
        let mut sessions = self.sessions.lock().await;
        sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionState {
                conversation_id,
                agent_id,
                model_id: None,
                cancel_notify: None,
            })
            .clone()
    }

    pub(super) fn send_notification(&self, notification: acp::SessionNotification) -> Result<()> {
        self.session_update_tx
            .try_send(notification)
            .map_err(|_| Error::Application(anyhow::anyhow!("Failed to send notification")))
    }
}

impl<S: crate::Services> AcpAdapter<S> {
    /// Creates a new ACP adapter and returns the notification receiver.
    pub(crate) fn new(
        services: Arc<S>,
    ) -> (Self, mpsc::Receiver<acp::SessionNotification>) {
        Self::with_services(services)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use forge_domain::{AgentId, ConversationId, ModelId};
    use tokio::sync::Notify;

    use super::{AcpAdapter, SessionState};

    #[tokio::test]
    async fn ensure_session_keeps_existing_state() {
        let adapter = AcpAdapter::new_for_test(());
        let conversation_id = ConversationId::generate();
        let notify = Arc::new(Notify::new());

        adapter
            .store_session(
                "session-1".to_string(),
                SessionState {
                    conversation_id: conversation_id.clone(),
                    agent_id: AgentId::new("original-agent"),
                    model_id: Some(ModelId::new("model-a")),
                    cancel_notify: Some(notify.clone()),
                },
            )
            .await;

        let actual = adapter
            .ensure_session(
                "session-1",
                ConversationId::generate(),
                AgentId::new("replacement-agent"),
            )
            .await;

        assert_eq!(actual.conversation_id, conversation_id);
        assert_eq!(actual.agent_id, AgentId::new("original-agent"));
        assert_eq!(actual.model_id, Some(ModelId::new("model-a")));
        assert!(actual.cancel_notify.is_some());
    }

    #[tokio::test]
    async fn ensure_session_creates_new_state_when_missing() {
        let adapter = AcpAdapter::new_for_test(());
        let conversation_id = ConversationId::generate();

        let actual = adapter
            .ensure_session(
                "new-session",
                conversation_id.clone(),
                AgentId::new("fresh-agent"),
            )
            .await;

        assert_eq!(actual.conversation_id, conversation_id);
        assert_eq!(actual.agent_id, AgentId::new("fresh-agent"));
        assert_eq!(actual.model_id, None);
        assert!(actual.cancel_notify.is_none());
    }

    #[tokio::test]
    async fn cancel_session_notifies_waiters() {
        let adapter = AcpAdapter::new_for_test(());
        let notify = Arc::new(Notify::new());
        let wait_for_cancel_handle = notify.clone();
        let wait_for_cancel = wait_for_cancel_handle.notified();

        adapter
            .store_session(
                "session-2".to_string(),
                SessionState {
                    conversation_id: ConversationId::generate(),
                    agent_id: AgentId::new("agent"),
                    model_id: None,
                    cancel_notify: Some(notify),
                },
            )
            .await;

        let cancelled = adapter.cancel_session("session-2").await;

        assert!(cancelled);
        let result = tokio::time::timeout(Duration::from_millis(100), wait_for_cancel).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cancel_session_returns_false_when_session_has_no_waiter() {
        let adapter = AcpAdapter::new_for_test(());

        let cancelled = adapter.cancel_session("missing-session").await;

        assert!(!cancelled);
    }

    #[tokio::test]
    async fn update_methods_change_existing_session() {
        let adapter = AcpAdapter::new_for_test(());
        let notify = Arc::new(Notify::new());

        adapter
            .store_session(
                "session-3".to_string(),
                SessionState {
                    conversation_id: ConversationId::generate(),
                    agent_id: AgentId::new("old-agent"),
                    model_id: None,
                    cancel_notify: None,
                },
            )
            .await;

        adapter
            .update_session_agent("session-3", AgentId::new("new-agent"))
            .await
            .unwrap();
        adapter
            .update_session_model("session-3", ModelId::new("new-model"))
            .await
            .unwrap();
        adapter
            .set_cancel_notify("session-3", Some(notify.clone()))
            .await
            .unwrap();

        let actual = adapter.session_state("session-3").await.unwrap();

        assert_eq!(actual.agent_id, AgentId::new("new-agent"));
        assert_eq!(actual.model_id, Some(ModelId::new("new-model")));
        assert!(actual.cancel_notify.is_some());
    }
}
