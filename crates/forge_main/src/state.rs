use std::path::PathBuf;
use std::time::Instant;

use derive_setters::Setters;
use forge_api::{ConversationId, Environment};

//TODO: UIState and ForgePrompt seem like the same thing and can be merged
/// State information for the UI
#[derive(Debug, Clone, Setters)]
#[setters(strip_option)]
pub struct UIState {
    pub cwd: PathBuf,
    pub conversation_id: Option<ConversationId>,
    pub goal: Option<String>,
    pub loop_enabled: bool,
    pub last_activity: Instant,
}

impl Default for UIState {
    fn default() -> Self {
        Self {
            cwd: PathBuf::from("."),
            conversation_id: None,
            goal: None,
            loop_enabled: false,
            last_activity: Instant::now(),
        }
    }
}

impl UIState {
    pub fn new(env: Environment) -> Self {
        Self { cwd: env.cwd, conversation_id: Default::default(), goal: None, loop_enabled: false, last_activity: Instant::now() }
    }
}
