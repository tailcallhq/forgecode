use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use forge_domain::{
    Conversation, EndPayload, EventData, EventHandle, StartPayload, ToolcallStartPayload,
};

/// The most recently constructed reporter, kept so that the process-wide
/// `release_global()` (called when forge exits) can release the Herdr pane
/// with a fresh sequence number even though the reporter itself lives inside
/// `ForgeApp::chat` and is rebuilt each turn.
static GLOBAL_REPORTER: OnceLock<Mutex<Option<HerdrReporter>>> = OnceLock::new();

fn global_reporter() -> &'static Mutex<Option<HerdrReporter>> {
    GLOBAL_REPORTER.get_or_init(|| Mutex::new(None))
}

/// Reports forge lifecycle state to a running Herdr server.
///
/// This is the official "Add Herdr support to your agent" integration: when
/// forge runs inside a Herdr pane, the pane exposes `HERDR_ENV`,
/// `HERDR_PANE_ID`, `HERDR_BIN_PATH` and `HERDR_SOCKET_PATH`. We use those to
/// report `working` / `idle` / `blocked` state so Herdr can show forge in the
/// sidebar, notify on completion, wait on state, and restore the same session
/// after a server restart.
///
/// Outside Herdr (`HERDR_ENV` unset) this handler is a no-op.
///
/// Reports are fire-and-forget: spawned in the background with a short
/// timeout, failures ignored, so Herdr can never slow forge down.
#[derive(Clone)]
pub struct HerdrReporter {
    /// Monotonic sequence number across reports and restarts. A timestamp is
    /// recommended by Herdr; out-of-order (lower) reports are ignored.
    seq: Arc<std::sync::atomic::AtomicU64>,
    /// The agent label Herdr shows in its sidebar. Use forge's own name.
    agent: &'static str,
    /// Stable, unique integration source id (must NOT start with `herdr:`).
    source: &'static str,
    /// Command that reopens the current session after a Herdr restart.
    resume_argv: Vec<String>,
}

/// The sequence base: a single monotonically increasing 64-bit counter seeded
/// from the process start so concurrent or restarted reporters don't collide.
fn fresh_seq_base() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1)
}

/// Whether we are inside a Herdr pane and a Herdr binary is available to talk
/// to.
fn herdr_env_available() -> bool {
    std::env::var_os("HERDR_ENV").is_some() && std::env::var_os("HERDR_BIN_PATH").is_some()
}

/// Runs `$HERDR_BIN_PATH pane ...` in the background so forge is never blocked
/// by a Herdr round-trip. Errors are intentionally swallowed.
fn spawn_herdr(mut args: Vec<String>) {
    use tokio::process::Command;

    if !herdr_env_available() {
        return;
    }
    let Some(bin) = std::env::var_os("HERDR_BIN_PATH") else {
        return;
    };
    let Some(pane_id) = std::env::var_os("HERDR_PANE_ID") else {
        return;
    };

    // Assemble: herdr pane <subcommand> <PANE_ID> [--arg value]...
    let Some(sub) = args.first().map(|s| s.as_str()) else {
        return;
    };
    let sub = match sub {
        "report-agent" => "report-agent",
        "report-agent-session" => "report-agent-session",
        "release-agent" => "release-agent",
        _ => return,
    };
    args.remove(0);
    let mut full = vec![
        bin.to_string_lossy().to_string(),
        "pane".to_string(),
        sub.to_string(),
        pane_id.to_string_lossy().to_string(),
    ];
    full.extend(args);

    tokio::spawn(async move {
        let _ = Command::new(&full[0])
            .args(&full[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await;
    });
}

impl HerdrReporter {
    pub fn new(agent_id: &str, conversation_id: &str, model_id: &str) -> Self {
        // Resume command must start with a plain command name on the user's
        // PATH and contain no apostrophes or control characters.
        let mut resume_argv = vec!["forge".to_string()];
        if !agent_id.is_empty() && agent_id != "forge" {
            resume_argv.push("--agent".to_string());
            resume_argv.push(agent_id.to_string());
        }
        if !conversation_id.is_empty() {
            resume_argv.push("--conversation-id".to_string());
            resume_argv.push(conversation_id.to_string());
        }
        if !model_id.is_empty() {
            resume_argv.push("--model".to_string());
            resume_argv.push(model_id.to_string());
        }
        let reporter = Self {
            seq: Arc::new(std::sync::atomic::AtomicU64::new(fresh_seq_base())),
            agent: "forge",
            source: "forge",
            resume_argv,
        };
        // Remember the freshest reporter for process-exit release.
        if let Ok(mut slot) = global_reporter().lock() {
            *slot = Some(reporter.clone());
        }
        reporter
    }

    fn next_seq(&self) -> u64 {
        use std::sync::atomic::Ordering;
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    /// Reports `working` / `idle` / `blocked`. When `with_resume` is set the
    /// resume command is attached so the pane can be restored after a Herdr
    /// restart.
    pub fn report_state(&self, state: &str, message: Option<&str>, with_resume: bool) {
        if !herdr_env_available() {
            return;
        }
        let seq = self.next_seq();
        let mut args = vec![
            "--source".to_string(),
            self.source.to_string(),
            "--agent".to_string(),
            self.agent.to_string(),
            "--state".to_string(),
            state.to_string(),
            "--seq".to_string(),
            seq.to_string(),
        ];
        if let Some(msg) = message {
            if !msg.is_empty() {
                args.push("--message".to_string());
                args.push(msg.to_string());
            }
        }
        if with_resume && !self.resume_argv.is_empty() {
            args.push("--".to_string());
            args.extend(self.resume_argv.iter().cloned());
        }
        args.insert(0, "report-agent".to_string());
        spawn_herdr(args);
    }

    /// Attaches/reports only the session identity when the conversation loads.
    /// Kept as API for future session-identity reporting (HERDR agent session
    /// restore flow); not currently wired into the hook chain.
    #[allow(dead_code)]
    pub fn report_session(&self, conversation_id: &str) {
        if !herdr_env_available() {
            return;
        }
        let seq = self.next_seq();
        let mut args = vec![
            "--source".to_string(),
            self.source.to_string(),
            "--agent".to_string(),
            self.agent.to_string(),
            "--state".to_string(),
            "working".to_string(),
            "--seq".to_string(),
            seq.to_string(),
            "--agent-session-id".to_string(),
            conversation_id.to_string(),
        ];
        if !self.resume_argv.is_empty() {
            args.push("--".to_string());
            args.extend(self.resume_argv.iter().cloned());
        }
        args.insert(0, "report-agent".to_string());
        spawn_herdr(args);
    }

    /// Releases the pane authority. Only called when forge actually exits.
    fn release(&self) {
        if !herdr_env_available() {
            return;
        }
        let seq = self.next_seq();
        let mut args = vec![
            "--source".to_string(),
            self.source.to_string(),
            "--agent".to_string(),
            self.agent.to_string(),
            "--seq".to_string(),
            seq.to_string(),
        ];
        args.insert(0, "release-agent".to_string());
        spawn_herdr(args);
    }
}

/// Releases the Herdr pane the current process was attached to. Safely a no-op
/// when forge runs outside Herdr. Called on normal process exit.
pub fn release_global() {
    if let Ok(slot) = global_reporter().lock() {
        if let Some(reporter) = slot.as_ref() {
            reporter.release();
        }
    }
}

/// Human-readable summary of a started tool call, used as the `blocked`
/// message when forge is waiting on user permission.
fn tool_summary(tool_call: &forge_domain::ToolCallFull) -> String {
    // ToolCallFull has `name` and `arguments`; keep the message short.
    let name = tool_call.name.as_str();
    let args = tool_call.arguments.clone().into_string();
    let truncated: String = args.chars().take(64).collect();
    if truncated.is_empty() {
        name.to_string()
    } else {
        format!("{name} {truncated}")
    }
}

#[async_trait]
impl EventHandle<EventData<StartPayload>> for HerdrReporter {
    async fn handle(
        &self,
        _event: &EventData<StartPayload>,
        _conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        // A turn started: report working and attach the resume command.
        self.report_state("working", None, true);
        Ok(())
    }
}

#[async_trait]
impl EventHandle<EventData<EndPayload>> for HerdrReporter {
    async fn handle(
        &self,
        _event: &EventData<EndPayload>,
        _conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        // A turn ended: back to idle (awaiting prompt). Not a release — forge
        // may keep running in interactive mode.
        self.report_state("idle", None, false);
        Ok(())
    }
}

#[async_trait]
impl EventHandle<EventData<ToolcallStartPayload>> for HerdrReporter {
    async fn handle(
        &self,
        event: &EventData<ToolcallStartPayload>,
        _conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        let summary = tool_summary(&event.payload.tool_call);
        self.report_state("working", Some(&summary), false);
        Ok(())
    }
}
