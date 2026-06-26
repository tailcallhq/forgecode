use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use derive_setters::Setters;
use forge_api::{ConversationId, Environment};
use forge_domain::ConversationSort;

//TODO: UIState and ForgePrompt seem like the same thing and can be merged
/// State information for the UI
#[derive(Debug, Setters)]
#[setters(strip_option)]
pub struct UIState {
    pub cwd: PathBuf,
    pub conversation_id: Option<ConversationId>,
    pub goal: Option<String>,
    pub loop_enabled: bool,
    pub last_activity: Instant,
    /// CWD filter for the conversation selector. When set, the selector
    /// scopes its results to conversations whose `cwd` column matches.
    /// This is the "filter by project directory" UX.
    pub cwd_filter: Option<String>,
    /// Sort key for the conversation selector. Re-exported from
    /// `forge_domain::ConversationSort` so there's one canonical enum
    /// across the repo / service / UI layers.
    pub sort: ConversationSort,
    /// Live status bar state (model, tokens, current tool, etc.).
    /// Wrapped in `Arc<Mutex<_>>` so the chat loop can update fields
    /// from the rendering thread without holding a `&mut` on `UI`.
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub status_bar: StatusBar,
    /// Global toggle for the compressed tool-output view.
    /// When `false` (the default), tool outputs are truncated to the
    /// first 3 lines + a "Ctrl+O to expand" hint. Pressing `Ctrl+O`
    /// flips this to `true` and the next tool output is shown in full.
    /// Tracks the latest tool call's expanded state by id, so toggling
    /// only affects the most recent tool output.
    pub tool_output_expanded: bool,
}

impl Default for UIState {
    fn default() -> Self {
        Self {
            cwd: PathBuf::from("."),
            conversation_id: None,
            goal: None,
            loop_enabled: false,
            last_activity: Instant::now(),
            cwd_filter: None,
            sort: ConversationSort::default(),
            status_bar: StatusBar::default(),
            tool_output_expanded: false,
        }
    }
}

/// Snapshot of `StatusBar` used by the renderer. All fields are
/// `Clone` and the snapshot is cheap to take (single `Mutex` lock).
// WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct StatusBarSnapshot {
    pub last_action: Option<String>,
    pub active_tool: Option<String>,
    pub context_pct: u8,
    pub tokens_used: u64,
    pub is_busy: bool,
    pub tool_in_flight: u32,
    pub active_tool_started: Option<Instant>,
}

impl StatusBarSnapshot {
    /// Elapsed time since the active tool started, if any.
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub fn active_tool_elapsed(&self) -> Option<Duration> {
        self.active_tool_started.map(|t| t.elapsed())
    }

    /// True when there is at least one in-flight tool call.
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub fn has_tool_in_flight(&self) -> bool {
        self.tool_in_flight > 0
    }
}

/// Live status-bar state, mutated by the chat loop and read by the
/// renderer. Use `snapshot()` to take a `StatusBarSnapshot` for display.
#[derive(Debug, Default)]
pub struct StatusBar {
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    inner: Mutex<StatusBarSnapshot>,
}

impl StatusBar {
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub fn snapshot(&self) -> StatusBarSnapshot {
        self.inner.lock().expect("StatusBar mutex poisoned").clone()
    }

    /// Set the last user-visible action (e.g. "edit: ui.rs:474").
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub fn set_last_action(&self, action: impl Into<String>) {
        let mut g = self.inner.lock().expect("StatusBar mutex poisoned");
        g.last_action = Some(action.into());
    }

    /// Set the current model id (e.g. "claude-sonnet-4").
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub fn set_model(&self, model: impl Into<String>) {
        self.set_last_action(format!("model: {}", model.into()));
    }

    /// Record a tool call start. Bumps `tool_in_flight` and records
    /// the active tool name and start time.
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub fn begin_tool(&self, name: impl Into<String>) {
        let mut g = self.inner.lock().expect("StatusBar mutex poisoned");
        g.active_tool = Some(name.into());
        g.active_tool_started = Some(Instant::now());
        g.tool_in_flight = g.tool_in_flight.saturating_add(1);
        g.is_busy = true;
    }

    /// Record a tool call finish. Decrements `tool_in_flight`; if it
    /// hits zero, clears the active tool.
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub fn end_tool(&self) {
        let mut g = self.inner.lock().expect("StatusBar mutex poisoned");
        g.tool_in_flight = g.tool_in_flight.saturating_sub(1);
        if g.tool_in_flight == 0 {
            g.active_tool = None;
            g.active_tool_started = None;
            g.is_busy = false;
        }
    }

    /// Update the token usage counters and derived context percentage.
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub fn set_tokens(&self, tokens_used: u64, context_pct: u8) {
        let mut g = self.inner.lock().expect("StatusBar mutex poisoned");
        g.tokens_used = tokens_used;
        g.context_pct = context_pct;
    }

    /// Mark the agent as busy (model thinking, no tool in flight).
    // WIP: Claude-style status bar (PRs #27/#29/#30), not yet fully wired into the render loop.
    #[allow(dead_code)]
    pub fn set_busy(&self, busy: bool) {
        let mut g = self.inner.lock().expect("StatusBar mutex poisoned");
        g.is_busy = busy;
    }
}

impl UIState {
    pub fn new(env: Environment) -> Self {
        Self {
            cwd: env.cwd,
            conversation_id: Default::default(),
            goal: None,
            loop_enabled: false,
            last_activity: Instant::now(),
            cwd_filter: None,
            sort: ConversationSort::default(),
            status_bar: StatusBar::new(),
            tool_output_expanded: false,
        }
    }
}
