use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use forge_api::{API, AgentId, Effort, Environment};

use crate::editor::{AgentState, EffortState, ForgeEditor, ReadResult};
use crate::model::{AppCommand, ForgeCommandManager};
use crate::prompt::ForgePrompt;
use crate::tracker;

pub struct Console {
    command: Arc<ForgeCommandManager>,
    editor: Mutex<ForgeEditor>,
    effort_state: Arc<Mutex<EffortState>>,
    agent_state: Arc<Mutex<AgentState>>,
}

impl Console {
    /// Creates a new instance of `Console`.
    pub fn new(
        env: Environment,
        custom_history_path: Option<PathBuf>,
        command: Arc<ForgeCommandManager>,
        current_agent: AgentId,
    ) -> Self {
        let effort_state = Arc::new(Mutex::new(EffortState::default()));
        let agent_state = Arc::new(Mutex::new(AgentState::new(current_agent)));
        let editor = Mutex::new(ForgeEditor::new(
            env,
            custom_history_path,
            command.clone(),
            effort_state.clone(),
            agent_state.clone(),
        ));
        Self { command, editor, effort_state, agent_state }
    }

    /// Returns a snapshot of the current effort, if set.
    pub fn current_effort(&self) -> Option<Effort> {
        self.effort_state.lock().unwrap().current.clone()
    }

    /// Returns a handle to the shared effort state for UI rendering.
    pub fn effort_state(&self) -> Arc<Mutex<EffortState>> {
        self.effort_state.clone()
    }

    /// Returns a handle to the shared agent state for UI rendering.
    pub fn agent_state(&self) -> Arc<Mutex<AgentState>> {
        self.agent_state.clone()
    }

    /// Updates the shared agent state to reflect a new active agent.
    pub fn set_agent(&self, agent_id: AgentId) {
        let mut state = self.agent_state.lock().unwrap();
        state.current = agent_id;
    }

    /// Low-level prompt that returns raw user input result
    pub fn prompt_raw(&self, prompt: &mut ForgePrompt) -> anyhow::Result<ReadResult> {
        let mut forge_editor = self.editor.lock().unwrap();
        forge_editor.prompt(prompt)
    }

    /// Refreshes the shared effort state from the API, then reads input.
    ///
    /// On every iteration we:
    /// 1. Resolve the active agent → model → supported efforts from the API.
    /// 2. Clamp the stored effort to the supported set (or clear it).
    /// 3. Read user input (Ctrl+T may cycle the effort in the editor).
    /// 4. Persist the (possibly changed) effort back to the API.
    /// 5. If Ctrl+Q cycled the agent, sync the new agent to the API before
    ///    processing the input returned by the same Enter keypress.
    pub async fn prompt<A: API>(
        &self,
        prompt: &mut ForgePrompt,
        api: &Arc<A>,
    ) -> anyhow::Result<AppCommand> {
        loop {
            self.sync_effort_from_api(api).await;

            let user_input = self.prompt_raw(prompt)?;

            self.sync_effort_to_api(api).await;

            // If Ctrl+Q cycled the agent, sync to API before handling the
            // input from this prompt. Do not continue here: when the user
            // switches mode and submits text in one prompt, that text must be
            // handled under the newly selected agent instead of being dropped.
            let cycled_agent = {
                let state = self.agent_state.lock().unwrap();
                state.current.clone()
            };
            if cycled_agent != prompt.agent_id {
                api.set_active_agent(cycled_agent.clone()).await?;
                prompt.agent_id = cycled_agent;
            }

            match user_input {
                ReadResult::Continue => continue,
                ReadResult::Exit => return Ok(AppCommand::Exit),
                ReadResult::Empty => continue,
                ReadResult::Success(text) => {
                    tracker::prompt(text.clone());
                    return self.command.parse(&text);
                }
            }
        }
    }

    /// Sets the buffer content for the next prompt
    pub fn set_buffer(&self, content: String) {
        let mut editor = self.editor.lock().unwrap();
        editor.set_buffer(content);
    }

    // -- Effort helpers -------------------------------------------------------

    /// Resolves the supported reasoning efforts for the active model and
    /// clamps the stored effort accordingly.
    async fn sync_effort_from_api<A: API>(&self, api: &Arc<A>) {
        let agent = match api.get_active_agent().await {
            Some(a) => a,
            None => return,
        };
        let model = match api.get_agent_model(agent).await {
            Some(m) => m,
            None => return,
        };

        // Only update when the API call succeeds; transient failures must not
        // wipe out the previously resolved supported list and current effort.
        if let Some(supported) = api.get_models().await.ok().and_then(|models| {
            models
                .into_iter()
                .find(|m| m.id == model)
                .map(|m| m.reasoning_efforts())
        }) {
            let api_effort = api.get_reasoning_effort().await.ok().flatten();

            let mut state = self.effort_state.lock().unwrap();
            state.supported = supported;
            state.current = Self::clamp_effort(api_effort, &state.supported);
        }
    }

    /// Clamps an effort to the supported set.
    ///
    /// - If the effort is in the supported set, keep it.
    /// - If the set is non-empty but doesn't contain the effort, fall back to
    ///   the first entry.
    /// - If the set is empty, clear the effort.
    fn clamp_effort(effort: Option<Effort>, supported: &[Effort]) -> Option<Effort> {
        match effort {
            Some(e) if supported.is_empty() => None,
            Some(e) if supported.contains(&e) => Some(e),
            Some(_) => supported.first().cloned(),
            None => None,
        }
    }

    /// Persists the current effort back to the API (if one is set).
    async fn sync_effort_to_api<A: API>(&self, api: &Arc<A>) {
        let effort = match self.current_effort() {
            Some(e) => e,
            None => return,
        };
        let _ = api
            .update_config(vec![forge_api::ConfigOperation::SetReasoningEffort(effort)])
            .await;
    }
}
