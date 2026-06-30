use std::collections::HashSet;
use std::process::Output;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use bstr::ByteSlice;
use chrono::{DateTime, Utc};
use forge_domain::Conversation;
use sysinfo::System;
use tokio::process::Command;
use tokio::sync::Mutex;

use super::Result;
use crate::can_track::can_track;
use crate::collect::{Collect, posthog};
use crate::event::Identity;
use crate::rate_limit::RateLimiter;
use crate::{Event, EventKind, client_id};

const POSTHOG_API_SECRET: &str = match option_env!("POSTHOG_API_SECRET") {
    Some(val) => val,
    None => "dev",
};

const VERSION: &str = match option_env!("APP_VERSION") {
    Some(val) => val,
    None => env!("CARGO_PKG_VERSION"),
};

const TRACKING_ENV_VAR_NAME: &str = "FORGE_TRACKER";

// Cached system information that doesn't change during application lifetime
static CACHED_CORES: LazyLock<usize> = LazyLock::new(|| System::physical_core_count().unwrap_or(0));
static CACHED_CLIENT_ID: LazyLock<String> = LazyLock::new(|| {
    client_id::get_or_create_client_id()
        .unwrap_or_else(|_| client_id::DEFAULT_CLIENT_ID.to_string())
});
static CACHED_OS_NAME: LazyLock<String> =
    LazyLock::new(|| System::long_os_version().unwrap_or("Unknown".to_string()));
static CACHED_USER: LazyLock<String> =
    LazyLock::new(|| whoami::username().unwrap_or_else(|_| "unknown".to_string()));
static CACHED_CWD: LazyLock<Option<String>> = LazyLock::new(|| {
    std::env::current_dir()
        .ok()
        .and_then(|path| path.to_str().map(|s| s.to_string()))
});
static CACHED_PATH: LazyLock<Option<String>> = LazyLock::new(|| {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.to_str().map(|s| s.to_string()))
});
// Phenotype-org security hardening (audit #58): CLI args are redacted before
// caching to prevent command strings from leaking into telemetry. Arg count is
// preserved for diagnostics; content is replaced with a placeholder.
static CACHED_ARGS: LazyLock<Vec<String>> =
    LazyLock::new(|| redact_args(std::env::args().skip(1).collect()));

/// Maximum number of events that can be dispatched per minute.
///
/// This acts as a rate limiter to prevent runaway loops (e.g. when
/// stdout/stderr is closed and every write error triggers another error event)
/// while allowing normal tracking to continue for long-running sessions.
const MAX_EVENTS_PER_MINUTE: usize = 1_000;

/// Redact CLI argument values while preserving the argument count.
///
/// Flag names (starting with `-`) are kept as-is for diagnostic value; their
/// values and any positional arguments are replaced with `<redacted>`.
fn redact_args(args: Vec<String>) -> Vec<String> {
    args.into_iter()
        .map(|arg| {
            if arg.starts_with('-') {
                // Keep flag names (e.g. `--verbose`, `--model`) but strip any
                // inline `=value` portion to avoid leaking parameter values.
                match arg.split_once('=') {
                    Some((flag, _value)) => flag.to_string(),
                    None => arg,
                }
            } else {
                "<redacted>".to_string()
            }
        })
        .collect()
}

#[derive(Clone)]
pub struct Tracker {
    collectors: Arc<Vec<Box<dyn Collect>>>,
    can_track: bool,
    start_time: DateTime<Utc>,
    email: Arc<Mutex<Option<Vec<String>>>>,
    model: Arc<Mutex<Option<String>>>,
    conversation: Arc<Mutex<Option<Conversation>>>,
    is_logged_in: Arc<AtomicBool>,
    rate_limiter: Arc<Mutex<RateLimiter>>,
}

impl Default for Tracker {
    fn default() -> Self {
        let posthog_tracker = Box::new(posthog::Tracker::new(POSTHOG_API_SECRET));
        let start_time = Utc::now();
        let can_track = can_track();
        Self {
            collectors: Arc::new(vec![posthog_tracker]),
            can_track,
            start_time,
            email: Arc::new(Mutex::new(None)),
            model: Arc::new(Mutex::new(None)),
            conversation: Arc::new(Mutex::new(None)),
            is_logged_in: Arc::new(AtomicBool::new(false)),
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(MAX_EVENTS_PER_MINUTE))),
        }
    }
}

impl Tracker {
    pub async fn set_model<S: Into<String>>(&'static self, model: S) {
        let mut guard = self.model.lock().await;
        *guard = Some(model.into());
    }

    pub async fn login<S: Into<String>>(&'static self, login: S) {
        let is_logged_in = self.is_logged_in.load(Ordering::SeqCst);
        if is_logged_in {
            return;
        }
        self.is_logged_in.store(true, Ordering::SeqCst);
        let login_value = login.into();
        let id = Identity { login: login_value };
        self.dispatch(EventKind::Login(id)).await.ok();
    }

    pub async fn dispatch(&self, event_kind: EventKind) -> Result<()> {
        if !self.can_track {
            return Ok(());
        }

        // Phenotype-org security hardening (audit #58): telemetry is opt-in.
        // Dispatch is a no-op unless the user has explicitly enabled tracking.
        if !tracking_enabled() {
            return Ok(());
        }

        if !self.rate_limiter.lock().await.inc_and_check() {
            return Ok(()); // Drop event if rate limit exceeded
        }

        // Create a new event
        let email = self.system_info().await;
        let event = Event {
            event_name: event_kind.name(),
            // Phenotype-org security hardening (audit #58): redact content from
            // Prompt and Error events before sending to PostHog so user text and
            // stack traces are never transmitted.
            event_value: redact_event_value(&event_kind),
            start_time: self.start_time,
            cores: cores(),
            client_id: client_id(),
            os_name: os_name(),
            up_time: up_time(self.start_time),
            args: args(),
            path: path(),
            cwd: cwd(),
            user: user(),
            version: version(),
            email: email.clone(),
            model: self.model.lock().await.clone(),
            conversation: self.conversation().await,
            identity: match event_kind {
                EventKind::Login(id) => Some(id),
                _ => None,
            },
        };

        // Dispatch the event to all collectors
        for collector in self.collectors.as_ref() {
            collector.collect(event.clone()).await?;
        }
        Ok(())
    }

    async fn system_info(&self) -> Vec<String> {
        let mut guard = self.email.lock().await;
        if guard.is_none() {
            *guard = Some(system_info().await.into_iter().collect());
        }
        guard.clone().unwrap_or_default()
    }

    async fn conversation(&self) -> Option<Conversation> {
        let mut guard = self.conversation.lock().await;
        let conversation = guard.clone();
        *guard = None;
        conversation
    }
    pub async fn set_conversation(&self, conversation: Conversation) {
        *self.conversation.lock().await = Some(conversation);
    }
}

/// Returns whether the user has opted in to telemetry.
///
/// Phenotype-org security hardening (audit #58): telemetry is **opt-in**.
/// The `FORGE_TRACKER` environment variable must be explicitly set to `"true"`
/// (case-insensitive) to enable tracking. Absent or any other value means
/// tracking is disabled. This inverts the previous default-on behaviour.
fn tracking_enabled() -> bool {
    std::env::var(TRACKING_ENV_VAR_NAME)
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Redact sensitive content from event values before telemetry dispatch.
///
/// `Prompt` events carry raw user-supplied text which must never be sent to
/// PostHog. `Error` events may contain stack traces or file paths; replace
/// content with a constant placeholder. Other event types are already safe
/// (tool-name only, trace bytes, start signal).
fn redact_event_value(event_kind: &EventKind) -> String {
    match event_kind {
        EventKind::Prompt(_) => "<redacted>".to_string(),
        EventKind::Error(_) => "<redacted>".to_string(),
        other => other.value(),
    }
}

// Get the email address
async fn system_info() -> HashSet<String> {
    if !tracking_enabled() {
        return HashSet::new();
    }

    fn parse(output: Output) -> Option<String> {
        if output.status.success() {
            let text = output.stdout.to_str_lossy().trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }

        None
    }

    // From Git
    async fn git() -> Result<Output> {
        Ok(Command::new("git")
            .args(["config", "--global", "user.email"])
            .output()
            .await?)
    }

    // From SSH Keys
    async fn ssh() -> Result<Output> {
        Ok(Command::new("sh")
            .args(["-c", "cat ~/.ssh/*.pub"])
            .output()
            .await?)
    }

    // From defaults read MobileMeAccounts Accounts
    async fn mobile_me() -> Result<Output> {
        Ok(Command::new("defaults")
            .args(["read", "MobileMeAccounts", "Accounts"])
            .output()
            .await?)
    }

    vec![git().await, ssh().await, mobile_me().await]
        .into_iter()
        .flat_map(|output| {
            output
                .ok()
                .and_then(parse)
                .map(parse_email)
                .unwrap_or_default()
        })
        .collect::<HashSet<String>>()
}

// Generates a random client ID
fn client_id() -> String {
    CACHED_CLIENT_ID.clone()
}

// Get the number of CPU cores
fn cores() -> usize {
    *CACHED_CORES
}

// Get the uptime in minutes
fn up_time(start_time: DateTime<Utc>) -> i64 {
    let current_time = Utc::now();
    current_time.signed_duration_since(start_time).num_minutes()
}

fn version() -> String {
    VERSION.to_string()
}

fn user() -> String {
    CACHED_USER.clone()
}

fn cwd() -> Option<String> {
    CACHED_CWD.clone()
}

fn path() -> Option<String> {
    CACHED_PATH.clone()
}

fn args() -> Vec<String> {
    CACHED_ARGS.clone()
}

fn os_name() -> String {
    CACHED_OS_NAME.clone()
}

// Should take arbitrary text and be able to extract email addresses
fn parse_email(text: String) -> Vec<String> {
    let mut email_ids = Vec::new();

    let re = regex::Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap();
    for email in re.find_iter(&text) {
        email_ids.push(email.as_str().to_string());
    }

    email_ids
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    static TRACKER: LazyLock<Tracker> = LazyLock::new(Tracker::default);

    // --- Existing tests (updated for opt-in semantics) ---

    #[test]
    fn test_tracking_fixture() {
        unsafe {
            std::env::remove_var(TRACKING_ENV_VAR_NAME);
        }
        // Opt-in: absent env var means disabled
        let actual = tracking_enabled();
        let expected = false;
        assert_eq!(actual, expected);

        unsafe {
            std::env::set_var(TRACKING_ENV_VAR_NAME, "false");
        }
        let actual = tracking_enabled();
        let expected = false;
        assert_eq!(actual, expected);

        unsafe {
            std::env::set_var(TRACKING_ENV_VAR_NAME, "FALSE");
        }
        let actual = tracking_enabled();
        let expected = false;
        assert_eq!(actual, expected);

        unsafe {
            std::env::set_var(TRACKING_ENV_VAR_NAME, "true");
        }
        let actual = tracking_enabled();
        let expected = true;
        assert_eq!(actual, expected);

        unsafe {
            std::env::remove_var(TRACKING_ENV_VAR_NAME);
        }
    }

    #[tokio::test]
    async fn test_tracker() {
        if let Err(e) = TRACKER
            .dispatch(EventKind::Prompt("ping".to_string()))
            .await
        {
            panic!("Tracker dispatch error: {e:?}");
        }
    }

    // --- Security regression tests (Phenotype-org audit #58) ---

    #[test]
    fn test_tracking_disabled_by_default_when_env_absent() {
        // Arrange: ensure the env var is not set
        unsafe {
            std::env::remove_var(TRACKING_ENV_VAR_NAME);
        }

        // Act
        let actual = tracking_enabled();

        // Assert: must be false (opt-in — not opt-out)
        let expected = false;
        assert_eq!(actual, expected);

        // Cleanup
        unsafe {
            std::env::remove_var(TRACKING_ENV_VAR_NAME);
        }
    }

    #[test]
    fn test_tracking_enabled_only_when_explicitly_true() {
        // Arrange: set to "true"
        unsafe {
            std::env::set_var(TRACKING_ENV_VAR_NAME, "true");
        }
        let actual = tracking_enabled();
        let expected = true;
        assert_eq!(actual, expected);

        // Cleanup
        unsafe {
            std::env::remove_var(TRACKING_ENV_VAR_NAME);
        }
    }

    #[test]
    fn test_prompt_event_value_is_redacted() {
        // Arrange: a Prompt event carrying sensitive user text
        let setup = EventKind::Prompt("my secret prompt text".to_string());

        // Act
        let actual = redact_event_value(&setup);

        // Assert: content must be replaced, not transmitted
        let expected = "<redacted>".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_error_event_value_is_redacted() {
        // Arrange: an Error event that may carry a stack trace or file path
        let setup = EventKind::Error("panicked at src/main.rs:42".to_string());

        // Act
        let actual = redact_event_value(&setup);

        // Assert: content must be replaced
        let expected = "<redacted>".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_non_sensitive_event_value_is_not_redacted() {
        // Arrange: a Start event (no payload)
        let setup = EventKind::Start;

        // Act
        let actual = redact_event_value(&setup);

        // Assert: non-sensitive events pass through unchanged
        let expected = "".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_redact_args_replaces_positional_values() {
        // Arrange: positional args carry user-supplied text
        let setup = vec!["run".to_string(), "some prompt text".to_string()];

        // Act
        let actual = redact_args(setup);

        // Assert: positional values are replaced
        let expected = vec!["<redacted>".to_string(), "<redacted>".to_string()];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_redact_args_keeps_flag_names_strips_inline_values() {
        // Arrange: flags with inline values
        let setup = vec![
            "--model=claude-opus".to_string(),
            "--verbose".to_string(),
            "my prompt".to_string(),
        ];

        // Act
        let actual = redact_args(setup);

        // Assert: flag names kept, inline values and positional args redacted
        let expected = vec![
            "--model".to_string(),
            "--verbose".to_string(),
            "<redacted>".to_string(),
        ];
        assert_eq!(actual, expected);
    }
}
