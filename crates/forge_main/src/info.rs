use std::fmt;
use std::path::Path;
use std::time::Duration;

use colored::Colorize;
use forge_api::{Conversation, Environment, ForgeConfig, Metrics, Role, Usage, UserUsage};
use forge_tracker::VERSION;
use num_format::{Locale, ToFormattedString};

use crate::display_constants::markers;
use crate::model::ForgeCommandManager;

#[derive(Debug, PartialEq)]
pub enum Section {
    Title(String),
    Items(Option<String>, String), // key, value
}

impl Section {
    pub fn key(&self) -> Option<&str> {
        match self {
            Section::Items(Some(key), _) => Some(key.as_str()),
            _ => None,
        }
    }
}

/// A structured information display builder for terminal output.
///
/// `Info` provides a consistent way to display hierarchical information with
/// titles, key-value pairs, and values. It handles alignment, formatting, and
/// color coding automatically.
///
/// # Display Conventions
///
/// When using Info, follow these conventions for consistency:
///
/// ## Keys (Labels)
/// - Use **Title Case** for keys (e.g., "Default Model", "API Key")
/// - Keep keys concise but descriptive
/// - Keys are automatically right-aligned within sections
/// - Keys are displayed in **green bold**
///
/// ## Values
/// - Use the constants from [`crate::display_constants`] for special values
/// - For empty values: Use `placeholders::EMPTY` (`[empty]`)
/// - For statuses: Use `status::ENABLED` (`[enabled]`) or `status::DISABLED`
///   (`[disabled]`)
/// - For actual values: Use the raw value (e.g., "gpt-4", "/home/user")
///
/// ## Sections
/// - Use **UPPERCASE** for section titles
/// - Section titles are displayed in **bold dimmed**
/// - Each section groups related key-value pairs
/// - Keys within a section are aligned to the longest key
///
/// # Examples
///
/// ```rust,ignore
/// use crate::display_constants::{placeholders, status};
/// use crate::info::Info;
///
/// let info = Info::new()
///     .add_title("CONFIGURATION")
///     .add_key_value("Model", "gpt-4")
///     .add_key_value("Provider", "openai")
///     .add_key_value("Status", status::ENABLED)
///     .add_title("METRICS")
///     .add_key_value("Tokens", "1000")
///     .add_key_value("Cost", "$0.02");
///
/// println!("{}", info);
/// ```
///
/// # Output Format
///
/// ```text
///
/// CONFIGURATION
///   model gpt-4
/// provider openai
///   status [enabled]
///
/// METRICS
/// tokens 1000
///   cost $0.02
/// ```
#[derive(Default)]
pub struct Info {
    sections: Vec<Section>,
}

impl Info {
    pub fn new() -> Self {
        Info { sections: Vec::new() }
    }

    /// Returns a reference to the sections
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Adds a section title to the info display.
    ///
    /// Section titles are displayed in UPPERCASE, bold, and dimmed. They group
    /// related key-value pairs that follow.
    ///
    /// # Convention
    /// - Always use UPPERCASE for section titles
    /// - Keep titles concise (e.g., "CONFIGURATION", "METRICS", "USER")
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let info = Info::new()
    ///     .add_title("ENVIRONMENT")  // Correct: UPPERCASE
    ///     .add_key_value("Path", "/home/user");
    /// ```
    pub fn add_title(mut self, title: impl ToString) -> Self {
        self.sections.push(Section::Title(title.to_string()));
        self
    }

    /// Adds a standalone value without a key (displayed as a bullet point).
    ///
    /// Values without keys are displayed with a bullet point (⦿) and indented.
    /// Use this for lists or grouped items under a section.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let info = Info::new()
    ///     .add_title("AVAILABLE TOOLS")
    ///     .add_value("read")
    ///     .add_value("write")
    ///     .add_value("shell");
    /// ```
    ///
    /// Output:
    /// ```text
    /// AVAILABLE TOOLS
    ///   ⦿ read
    ///   ⦿ write
    ///   ⦿ shell
    /// ```
    pub fn add_value(self, value: impl IntoInfoValue) -> Self {
        self.add_item(None::<String>, value)
    }

    /// Adds a key without a value (displays as a label).
    ///
    /// This is typically used for labels or empty fields. The key is displayed
    /// without any value following it.
    ///
    /// # Convention
    /// - Use **Title Case** for keys (same as
    ///   [`add_key_value`](Self::add_key_value))
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let info = Info::new()
    ///     .add_title("EMPTY FIELDS")
    ///     .add_key("API Key");  // Shows just "api key" without a value
    /// ```
    pub fn add_key(self, key: impl ToString) -> Self {
        self.add_key_value(key, None::<String>)
    }

    /// Adds a key-value pair to the info display.
    ///
    /// The key is displayed in green bold, and the value follows it. Keys are
    /// automatically aligned within each section.
    ///
    /// # Conventions
    ///
    /// ## Keys
    /// - Use **Title Case** (e.g., "Default Model", "API Key", "Working
    ///   Directory")
    /// - Be descriptive but concise
    /// - Avoid abbreviations unless widely known
    ///
    /// ## Values
    /// - Use the constants from [`crate::display_constants`] for special
    ///   values:
    ///   - `placeholders::EMPTY` - empty value (`[empty]`)
    ///   - `status::ENABLED` - enabled/configured (`[enabled]`)
    ///   - `status::DISABLED` - disabled (`[disabled]`)
    /// - For actual values: Use the raw value (e.g., "gpt-4", "/home/user")
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use crate::display_constants::status;
    ///
    /// let info = Info::new()
    ///     .add_title("CONFIGURATION")
    ///     // Correct: Raw value for actual data
    ///     .add_key_value("Default Model", "gpt-4")
    ///     // Correct: Status constant for status values
    ///     .add_key_value("Provider Status", status::ENABLED)
    ///     // Correct: Raw value for actual data
    ///     .add_key_value("API URL", "https://api.example.com");
    /// ```
    ///
    /// # Incorrect Usage
    ///
    /// ```rust,ignore
    /// // ❌ Wrong: lowercase key
    /// .add_key_value("model", "gpt-4")
    ///
    /// // ❌ Wrong: raw string instead of constant
    /// .add_key_value("Status", "[enabled]")
    ///
    /// // ✅ Correct: Title Case key with constant
    /// .add_key_value("Status", status::ENABLED)
    /// ```
    pub fn add_key_value(self, key: impl ToString, value: impl IntoInfoValue) -> Self {
        let key_str = key.to_string();
        let normalized_key = key_str.to_lowercase();
        self.add_item(Some(normalized_key), value)
    }

    fn add_item(mut self, key: Option<impl ToString>, value: impl IntoInfoValue) -> Self {
        self.sections.push(Section::Items(
            key.map(|a| a.to_string()),
            value.into_value().unwrap_or(markers::EMPTY.to_string()),
        ));
        self
    }

    pub fn extend(mut self, other: impl Into<Info>) -> Self {
        self.sections.extend(other.into().sections);
        self
    }
}

pub trait IntoInfoValue {
    fn into_value(self) -> Option<String>;
}

impl IntoInfoValue for &str {
    fn into_value(self) -> Option<String> {
        Some(self.to_string())
    }
}

impl IntoInfoValue for &String {
    fn into_value(self) -> Option<String> {
        Some(self.to_owned())
    }
}

impl IntoInfoValue for String {
    fn into_value(self) -> Option<String> {
        Some(self)
    }
}

impl IntoInfoValue for crate::display_constants::CommandType {
    fn into_value(self) -> Option<String> {
        Some(self.to_string())
    }
}

impl<T: IntoInfoValue> IntoInfoValue for Option<T> {
    fn into_value(self) -> Option<String> {
        self.and_then(|o| o.into_value())
    }
}

impl From<&Environment> for Info {
    fn from(env: &Environment) -> Self {
        // Get the current git branch
        let branch_info = match get_git_branch() {
            Some(branch) => branch,
            None => "(not in a git repository)".to_string(),
        };

        let mut info = Info::new()
            .add_title("ENVIRONMENT")
            .add_key_value("Version", VERSION)
            .add_key_value("Working Directory", format_path_for_display(env, &env.cwd))
            .add_key_value("Shell", env.shell.as_str())
            .add_key_value("Git Branch", branch_info)
            .add_title("PATHS");

        // Only show logs path if the directory exists
        let log_path = env.log_path();
        if log_path.exists() {
            info = info.add_key_value("Logs", format_path_for_display(env, &log_path));
        }

        let agent_path = env.agent_path();
        info = info
            .add_key_value("Agents", format_path_for_display(env, &agent_path))
            .add_key_value(
                "History",
                format_path_for_display(env, &env.history_path(None)),
            )
            .add_key_value(
                "Checkpoints",
                format_path_for_display(env, &env.snapshot_path()),
            )
            .add_key_value(
                "Policies",
                format_path_for_display(env, &env.permissions_path()),
            );

        info
    }
}

impl From<&ForgeConfig> for Info {
    fn from(config: &ForgeConfig) -> Self {
        let mut info = Info::new();

        // RETRY CONFIGURATION
        if let Some(retry) = &config.retry {
            info = info
                .add_title("RETRY CONFIGURATION")
                .add_key_value("Initial Backoff", format!("{}ms", retry.initial_backoff_ms))
                .add_key_value("Backoff Factor", retry.backoff_factor.to_string())
                .add_key_value("Max Attempts", retry.max_attempts.to_string())
                .add_key_value("Suppress Errors", retry.suppress_errors.to_string())
                .add_key_value(
                    "Status Codes",
                    retry
                        .status_codes
                        .iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
        }

        // HTTP CONFIGURATION
        if let Some(http) = &config.http {
            info = info
                .add_title("HTTP CONFIGURATION")
                .add_key_value("Connect Timeout", format!("{}s", http.connect_timeout_secs))
                .add_key_value("Read Timeout", format!("{}s", http.read_timeout_secs))
                .add_key_value(
                    "Pool Idle Timeout",
                    format!("{}s", http.pool_idle_timeout_secs),
                )
                .add_key_value("Pool Max Idle", http.pool_max_idle_per_host.to_string())
                .add_key_value("Max Redirects", http.max_redirects.to_string())
                .add_key_value("Use Hickory DNS", http.hickory.to_string())
                .add_key_value("TLS Backend", format!("{:?}", http.tls_backend))
                .add_key_value(
                    "Min TLS Version",
                    http.min_tls_version.as_ref().map(|v| format!("{v:?}")),
                )
                .add_key_value(
                    "Max TLS Version",
                    http.max_tls_version.as_ref().map(|v| format!("{v:?}")),
                )
                .add_key_value("Adaptive Window", http.adaptive_window.to_string())
                .add_key_value(
                    "Keep-Alive Interval",
                    http.keep_alive_interval_secs.map(|v| format!("{v}s")),
                )
                .add_key_value(
                    "Keep-Alive Timeout",
                    format!("{}s", http.keep_alive_timeout_secs),
                )
                .add_key_value(
                    "Keep-Alive While Idle",
                    http.keep_alive_while_idle.to_string(),
                )
                .add_key_value(
                    "Accept Invalid Certs",
                    http.accept_invalid_certs.to_string(),
                )
                .add_key_value(
                    "Root Cert Paths",
                    http.root_cert_paths
                        .as_ref()
                        .map(|paths| paths.join(", "))
                        .unwrap_or_else(|| markers::EMPTY.to_string()),
                );
        }

        info = info
            .add_title("API CONFIGURATION")
            .add_key_value("ForgeCode Service URL", config.services_url.to_string())
            .add_title("TOOL CONFIGURATION")
            .add_key_value("Tool Timeout", format!("{}s", config.tool_timeout_secs))
            .add_key_value(
                "Max Image Size",
                format!("{} bytes", config.max_image_size_bytes),
            )
            .add_key_value("Auto Open Dump", config.auto_open_dump.to_string())
            .add_key_value(
                "Debug Requests",
                config
                    .debug_requests
                    .as_ref()
                    .map(|p| p.display().to_string()),
            )
            .add_key_value(
                "Stdout Max Line Length",
                config.max_stdout_line_chars.to_string(),
            )
            .add_title("SYSTEM CONFIGURATION")
            .add_key_value(
                "Max Search Result Bytes",
                format!("{} bytes", config.max_search_result_bytes),
            )
            .add_key_value("Max Conversations", config.max_conversations.to_string());

        info
    }
}

impl From<&Metrics> for Info {
    fn from(metrics: &Metrics) -> Self {
        let mut info = Info::new();
        if let Some(duration) = metrics.duration(chrono::Utc::now())
            && duration.as_secs() > 0
        {
            let duration =
                humantime::format_duration(Duration::from_secs(duration.as_secs())).to_string();
            info = info.add_title(format!("TASK COMPLETED [in {duration}]"));
        } else {
            info = info.add_title("TASK COMPLETED".to_string())
        }

        // Add file changes section, filtering out files with minimal changes
        let meaningful_changes: Vec<_> = metrics
            .file_operations
            .iter()
            .filter(|(_, file_metrics)| {
                // Only show files with actual changes
                file_metrics.lines_added > 0 || file_metrics.lines_removed > 0
            })
            .collect();

        if meaningful_changes.is_empty() {
            info = info.add_value("[No Changes Produced]");
        } else {
            for (path, file_metrics) in meaningful_changes {
                // Extract just the filename from the path
                let filename = std::path::Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(path);

                let removed = if file_metrics.lines_removed == 0 {
                    "0".to_string()
                } else {
                    format!("−{}", file_metrics.lines_removed)
                };
                let added = if file_metrics.lines_added == 0 {
                    "0".to_string()
                } else {
                    format!("+{}", file_metrics.lines_added)
                };
                let changes = format!("{} {}", removed, added);

                info = info.add_key_value(filename, changes);
            }
        }

        info
    }
}

impl From<&Usage> for Info {
    fn from(value: &Usage) -> Self {
        let cache_percentage = calculate_cache_percentage(value);
        let cached_display = if cache_percentage > 0 {
            format!(
                "{} [{}%]",
                value.cached_tokens.to_formatted_string(&Locale::en),
                cache_percentage
            )
        } else {
            value.cached_tokens.to_formatted_string(&Locale::en)
        };

        let mut usage_info = Info::new()
            .add_title("TOKEN USAGE")
            .add_key_value(
                "Input Tokens",
                value.prompt_tokens.to_formatted_string(&Locale::en),
            )
            .add_key_value("Cached Tokens", cached_display)
            .add_key_value(
                "Output Tokens",
                value.completion_tokens.to_formatted_string(&Locale::en),
            );

        if let Some(cost) = value.cost.as_ref() {
            usage_info = usage_info.add_key_value("Cost", format!("${cost:.4}"));
        }
        usage_info
    }
}

fn calculate_cache_percentage(usage: &Usage) -> u8 {
    let total = *usage.prompt_tokens; // Use prompt tokens as the base for cache percentage
    let cached = *usage.cached_tokens;
    (cached * 100).checked_div(total).unwrap_or(0) as u8
}

impl fmt::Display for Info {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut width: Option<usize> = None;

        for (i, section) in self.sections.iter().enumerate() {
            match section {
                Section::Title(title) => {
                    writeln!(f)?;
                    writeln!(f, "{}", title.bold().dimmed())?;

                    // Calculate max key width for items under this title
                    width = self
                        .sections
                        .iter()
                        .skip(i + 1)
                        .take_while(|s| matches!(s, Section::Items(..)))
                        .filter_map(|s| s.key())
                        .map(|key| key.len())
                        .max();
                }
                Section::Items(key, value) => match (key.as_ref(), width) {
                    (Some(k), Some(w)) => {
                        writeln!(f, "  {} {}", format!("{k:<w$}").green().bold(), value)?;
                    }
                    (Some(k), None) => {
                        writeln!(f, "  {} {}", k.green().bold(), value)?;
                    }
                    (None, _) => {
                        writeln!(f, "    {} {}", "⦿".green(), value)?;
                    }
                },
            }
        }
        Ok(())
    }
}

/// Formats a path for display, using actual home directory on Windows and tilde
/// notation on Unix, with proper quoting for paths containing spaces
pub(crate) fn format_path_for_display(env: &Environment, path: &Path) -> String {
    // Check if path is under home directory first
    if let Some(home) = &env.home
        && let Ok(rel_path) = path.strip_prefix(home)
    {
        // Format based on OS
        return if env.os == "windows" {
            // Use actual home path with proper quoting for Windows to work in both cmd and
            // PowerShell
            let home_path = home.display().to_string();
            let full_path = format!(
                "{}{}{}",
                home_path,
                std::path::MAIN_SEPARATOR,
                rel_path.display()
            );
            if full_path.contains(' ') {
                format!("\"{full_path}\"")
            } else {
                full_path
            }
        } else {
            format!("~/{}", rel_path.display())
        };
    }

    // Fall back to absolute path if not under home directory
    // Quote paths on Windows if they contain spaces
    let path_str = path.display().to_string();
    if env.os == "windows" && path_str.contains(' ') {
        format!("\"{path_str}\"")
    } else {
        path_str
    }
}

/// Gets the current git branch name if available
fn get_git_branch() -> Option<String> {
    // First check if we're in a git repository
    let git_check = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .ok()?;

    if !git_check.status.success() || git_check.stdout.is_empty() {
        return None;
    }

    // If we are in a git repo, get the branch
    let output = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()?;

    if output.status.success() {
        String::from_utf8(output.stdout)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    }
}

/// Create an info instance for available commands from a ForgeCommandManager
impl From<&ForgeCommandManager> for Info {
    fn from(command_manager: &ForgeCommandManager) -> Self {
        let mut info = Info::new().add_title("COMMANDS");

        for command in command_manager.list() {
            info = info.add_key_value(command.name, command.description);
        }

        // Use compile-time OS detection for keyboard shortcuts
        #[cfg(target_os = "macos")]
        let multiline_shortcut = "<OPT+ENTER>";

        #[cfg(not(target_os = "macos"))]
        let multiline_shortcut = "<ALT+ENTER>";

        info = info
            .add_title("KEYBOARD SHORTCUTS")
            .add_key_value("<CTRL+C>", "Interrupt current operation")
            .add_key_value("<CTRL+D>", "Quit Forge interactive shell")
            .add_key_value(multiline_shortcut, "Insert new line (multiline input)");

        info
    }
}
impl From<&UserUsage> for Info {
    fn from(user_usage: &UserUsage) -> Self {
        let usage = &user_usage.usage;
        let plan = &user_usage.plan;

        let mut info = Info::new().add_title("REQUEST QUOTA");

        if plan.is_upgradeable() {
            info = info.add_key_value(
                "Subscription",
                format!(
                    "{} [Upgrade https://app.forgecode.dev/app/billing]",
                    plan.r#type.to_uppercase()
                ),
            );
        } else {
            info = info.add_key_value("Subscription", plan.r#type.to_uppercase());
        }

        info = info.add_key_value(
            "Usage",
            format!(
                "{} / {} [{} Remaining]",
                usage.current, usage.limit, usage.remaining
            ),
        );

        // Create progress bar for usage visualization
        let progress_bar = create_progress_bar(usage.current, usage.limit, 20);

        // Add reset information if available
        if let Some(reset_in) = usage.reset_in {
            info = info.add_key_value("Resets in", format_reset_time(reset_in));
        }

        info.add_key_value("Progress", progress_bar)
    }
}

pub fn create_progress_bar(current: u32, limit: u32, width: usize) -> String {
    if limit == 0 {
        return "N/A".to_string();
    }

    let percentage = (current as f64 / limit as f64 * 100.0).min(100.0);
    let filled_chars = ((current as f64 / limit as f64) * width as f64).round() as usize;
    let filled_chars = filled_chars.min(width);
    let empty_chars = width - filled_chars;

    // Option 1: Unicode block characters (most visually appealing)
    format!(
        "▐{}{} {:.1}%",
        "█".repeat(filled_chars),
        "░".repeat(empty_chars),
        percentage
    )
}

pub fn format_reset_time(seconds: u64) -> String {
    if seconds == 0 {
        return "now".to_string();
    }
    humantime::format_duration(Duration::from_secs(seconds)).to_string()
}

/// Extracts the first line of raw content from a context message.
fn format_user_message(msg: &forge_api::ContextMessage) -> Option<String> {
    let content = msg
        .as_value()
        .and_then(|v| v.as_user_prompt())
        .map(|p| p.as_str())?;
    let trimmed = content.lines().next().unwrap_or(content);
    Some(trimmed.to_string())
}

impl From<&Conversation> for Info {
    fn from(conversation: &Conversation) -> Self {
        let mut info = Info::new().add_title("CONVERSATION");

        info = info.add_key_value("ID", conversation.id.to_string());

        if let Some(title) = &conversation.title {
            info = info.add_key_value("Title", title);
        }

        // Add task and feedback (if available)

        let mut user_messages = conversation
            .context
            .iter()
            .flat_map(|ctx| ctx.messages.iter())
            .filter(|message| message.has_role(Role::User));

        let task = user_messages.next();

        if let Some(task) = task
            && let Some(task) = format_user_message(task)
        {
            info = info.add_key_value("Tasks", task);

            for feedback in user_messages {
                if let Some(feedback) = format_user_message(feedback) {
                    info = info.add_value(feedback);
                }
            }
        }

        // Insert metrics information
        if !conversation.metrics.file_operations.is_empty() {
            info = info.extend(&conversation.metrics);
        }

        // Insert token usage
        if let Some(usage) = conversation.accumulated_usage().as_ref() {
            info = info.extend(usage);
        }

        info
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use forge_api::{Environment, EventValue};
    use pretty_assertions::assert_eq;

    // Helper to create minimal test environment
    fn create_env(os: &str, home: Option<&str>) -> Environment {
        use fake::{Fake, Faker};
        let mut fixture: Environment = Faker.fake();
        fixture = fixture.os(os.to_string());
        if let Some(home_path) = home {
            fixture = fixture.home(PathBuf::from(home_path));
        }
        fixture
    }

    #[test]
    fn test_format_path_for_display_unix_home() {
        let fixture = create_env("linux", Some("/home/user"));
        let path = PathBuf::from("/home/user/project");

        let actual = super::format_path_for_display(&fixture, &path);
        let expected = "~/project";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_path_for_display_windows_home() {
        let fixture = create_env("windows", Some("C:\\Users\\User"));
        let path = PathBuf::from("C:\\Users\\User\\project");

        let actual = super::format_path_for_display(&fixture, &path);
        let expected = "C:\\Users\\User\\project";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_path_for_display_windows_home_with_spaces() {
        let fixture = create_env("windows", Some("C:\\Users\\User Name"));
        let path = PathBuf::from("C:\\Users\\User Name\\project");

        let actual = super::format_path_for_display(&fixture, &path);
        let expected = "\"C:\\Users\\User Name\\project\"";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_path_for_display_absolute() {
        let fixture = create_env("linux", Some("/home/user"));
        let path = PathBuf::from("/var/log/app");

        let actual = super::format_path_for_display(&fixture, &path);
        let expected = "/var/log/app";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_path_for_display_absolute_windows_with_spaces() {
        let fixture = create_env("windows", Some("C:/Users/User"));
        let path = PathBuf::from("C:/Program Files/App");

        let actual = super::format_path_for_display(&fixture, &path);
        let expected = "\"C:/Program Files/App\"";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_progress_bar() {
        // Test normal case - 70% of 20 = 14 filled, 6 empty
        let actual = super::create_progress_bar(70, 100, 20);
        let expected = "▐██████████████░░░░░░ 70.0%";
        assert_eq!(actual, expected);

        // Test 100% case
        let actual = super::create_progress_bar(100, 100, 20);
        let expected = "▐████████████████████ 100.0%";
        assert_eq!(actual, expected);

        // Test 0% case
        let actual = super::create_progress_bar(0, 100, 20);
        let expected = "▐░░░░░░░░░░░░░░░░░░░░ 0.0%";
        assert_eq!(actual, expected);

        // Test zero limit case
        let actual = super::create_progress_bar(50, 0, 20);
        let expected = "N/A";
        assert_eq!(actual, expected);

        // Test over 100% case (should cap at 100%)
        let actual = super::create_progress_bar(150, 100, 20);
        let expected = "▐████████████████████ 100.0%";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_path_for_display_no_home() {
        let fixture = create_env("linux", None);
        let path = PathBuf::from("/home/user/project");

        let actual = super::format_path_for_display(&fixture, &path);
        let expected = "/home/user/project";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_reset_time_hours_and_minutes() {
        let actual = super::format_reset_time(3661); // 1 hour, 1 minute, 1 second
        let expected = "1h 1m 1s";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_reset_time_hours_only() {
        let actual = super::format_reset_time(3600); // exactly 1 hour
        let expected = "1h";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_reset_time_minutes_and_seconds() {
        let actual = super::format_reset_time(125); // 2 minutes, 5 seconds
        let expected = "2m 5s";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_reset_time_minutes_only() {
        let actual = super::format_reset_time(120); // exactly 2 minutes
        let expected = "2m";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_reset_time_seconds_only() {
        let actual = super::format_reset_time(45); // 45 seconds
        let expected = "45s";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_reset_time_zero() {
        let actual = super::format_reset_time(0);
        let expected = "now";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_reset_time_large_value() {
        let actual = super::format_reset_time(7265); // 2 hours, 1 minute, 5 seconds
        let expected = "2h 1m 5s";
        assert_eq!(actual, expected);
    }
    #[test]
    fn test_metrics_info_display() {
        use forge_api::Metrics;
        use forge_domain::{FileOperation, ToolKind};

        let fixture = Metrics::default()
            .started_at(chrono::Utc::now())
            .insert(
                "src/main.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(12u64)
                    .lines_removed(3u64),
            )
            .insert(
                "src/agent/mod.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(8u64)
                    .lines_removed(2u64),
            )
            .insert(
                "tests/integration/test_agent.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(5u64)
                    .lines_removed(0u64),
            );

        let actual = super::Info::from(&fixture);
        let expected_display = actual.to_string();

        // Verify it contains the task completed section
        assert!(expected_display.contains("TASK COMPLETED"));

        // Verify it contains the files as keys with colons
        assert!(expected_display.contains("main.rs"));
        assert!(expected_display.contains("−3 +12"));
        assert!(expected_display.contains("mod.rs"));
        assert!(expected_display.contains("−2 +8"));
        assert!(expected_display.contains("test_agent.rs"));
        assert!(expected_display.contains("0 +5"));
    }

    #[test]
    fn test_conversation_info_display() {
        use chrono::Utc;
        use forge_api::ConversationId;
        use forge_domain::{FileOperation, ToolKind};

        use super::{Conversation, Metrics};

        let conversation_id = ConversationId::generate();
        let metrics = Metrics::default()
            .started_at(Utc::now())
            .insert(
                "src/main.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(5u64)
                    .lines_removed(2u64),
            )
            .insert(
                "tests/test.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(3u64)
                    .lines_removed(1u64),
            );

        let fixture = Conversation {
            id: conversation_id,
            title: Some("Test Conversation".to_string()),
            context: None,
            metrics,
            metadata: forge_domain::MetaData::new(Utc::now()),
        };

        let actual = super::Info::from(&fixture);
        let expected_display = actual.to_string();

        // Verify it contains the conversation section
        assert!(expected_display.contains("CONVERSATION"));
        assert!(expected_display.contains("Test Conversation"));
        assert!(expected_display.contains(&conversation_id.to_string()));
    }

    #[test]
    fn test_conversation_info_display_untitled() {
        use chrono::Utc;
        use forge_api::ConversationId;

        use super::{Conversation, Metrics};

        let conversation_id = ConversationId::generate();
        let metrics = Metrics::default().started_at(Utc::now());

        let fixture = Conversation {
            id: conversation_id,
            title: None,
            context: None,
            metrics,
            metadata: forge_domain::MetaData::new(Utc::now()),
        };

        let actual = super::Info::from(&fixture);
        let expected_display = actual.to_string();

        // Verify it contains the conversation section with untitled
        assert!(expected_display.contains("CONVERSATION"));
        assert!(!expected_display.contains("Title:"));
        assert!(expected_display.contains(&conversation_id.to_string()));
    }

    #[test]
    fn test_conversation_info_display_with_task() {
        use chrono::Utc;
        use forge_api::{Context, ContextMessage, ConversationId, Role};

        use super::{Conversation, Metrics};

        let conversation_id = ConversationId::generate();
        let metrics = Metrics::default().started_at(Utc::now());

        // Create a context with user messages
        let context = Context::default()
            .add_message(ContextMessage::system("System prompt"))
            .add_message(ContextMessage::Text(
                forge_domain::TextMessage::new(Role::User, "First user message")
                    .raw_content(EventValue::text("First user message")),
            ))
            .add_message(ContextMessage::assistant(
                "Assistant response",
                None,
                None,
                None,
            ))
            .add_message(ContextMessage::Text(
                forge_domain::TextMessage::new(Role::User, "Create a new feature")
                    .raw_content(EventValue::text("Create a new feature")),
            ));

        let fixture = Conversation {
            id: conversation_id,
            title: Some("Test Task".to_string()),
            context: Some(context),
            metrics,
            metadata: forge_domain::MetaData::new(Utc::now()),
        };

        let actual = super::Info::from(&fixture);
        let expected_display = actual.to_string();

        // Verify it contains the conversation section with task
        assert!(expected_display.contains("CONVERSATION"));
        assert!(expected_display.contains("Test Task"));
        // Check for Task separately due to ANSI color codes
        assert!(expected_display.contains("Task"));
        assert!(expected_display.contains("Create a new feature"));
        assert!(expected_display.contains(&conversation_id.to_string()));
    }

    #[test]
    fn test_info_display_with_consistent_key_padding() {
        use super::Info;

        let fixture = Info::new()
            .add_title("SECTION ONE")
            .add_key_value("Short", "value1")
            .add_key_value("Very Long Key", "value2")
            .add_key_value("Mid", "value3")
            .add_title("SECTION TWO")
            .add_key_value("A", "valueA")
            .add_key_value("ABC", "valueB");

        let actual = fixture.to_string();

        // Strip ANSI codes for easier assertion
        let stripped = strip_ansi_escapes::strip(&actual);
        let actual_str = String::from_utf8(stripped).unwrap();

        // Verify that keys are padded within each section
        // In SECTION ONE, all keys should be padded to length of "Very Long Key" (13)
        // In SECTION TWO, all keys should be padded to length of "ABC" (3)

        // Check that the display contains properly formatted sections
        assert!(actual_str.contains("SECTION ONE"));
        assert!(actual_str.contains("SECTION TWO"));

        // Verify padding by checking alignment of values
        // All keys in a section should have values starting at the same column
        let lines: Vec<&str> = actual_str.lines().collect();

        // Find SECTION ONE items
        let section_one_start = lines
            .iter()
            .position(|l| l.contains("SECTION ONE"))
            .unwrap();
        let section_two_start = lines
            .iter()
            .position(|l| l.contains("SECTION TWO"))
            .unwrap();

        let section_one_items: Vec<&str> = lines[section_one_start + 1..section_two_start]
            .iter()
            .filter(|l| !l.trim().is_empty() && !l.contains("SECTION"))
            .copied()
            .collect();

        // All values in section one should start at the same position
        // Find where "value" starts in each line
        let value_positions: Vec<usize> = section_one_items
            .iter()
            .map(|line| line.find("value").unwrap())
            .collect();

        assert!(
            value_positions.windows(2).all(|w| w[0] == w[1]),
            "Values in SECTION ONE should be aligned. Value positions: {:?}",
            value_positions
        );

        // Check SECTION TWO items
        let section_two_items: Vec<&str> = lines[section_two_start + 1..]
            .iter()
            .filter(|l| !l.trim().is_empty() && !l.contains("SECTION"))
            .copied()
            .collect();

        let value_positions_two: Vec<usize> = section_two_items
            .iter()
            .map(|line| line.find("value").unwrap())
            .collect();

        assert!(
            value_positions_two.windows(2).all(|w| w[0] == w[1]),
            "Values in SECTION TWO should be aligned. Value positions: {:?}",
            value_positions_two
        );

        // Verify that different sections can have different padding
        // (SECTION ONE should have wider padding than SECTION TWO)
        assert!(
            value_positions[0] > value_positions_two[0],
            "SECTION ONE should have wider padding than SECTION TWO"
        );
    }

    #[test]
    fn test_add_key_value_normalizes_to_lowercase() {
        let info = super::Info::new()
            .add_key_value("VERSION", "1.0.0")
            .add_key_value("Working Directory", "/home/user")
            .add_key_value("Mixed CASE Key", "value");

        let display = info.to_string();

        // All keys should be lowercase - checking just the key part without exact
        // formatting
        assert!(display.contains("version"));
        assert!(display.contains("working directory"));
        assert!(display.contains("mixed case key"));

        // Values should be preserved
        assert!(display.contains("1.0.0"));
        assert!(display.contains("/home/user"));
        assert!(display.contains("value"));

        // Should not contain uppercase versions
        assert!(!display.contains("VERSION"));
        assert!(!display.contains("Working Directory"));
        assert!(!display.contains("Mixed CASE Key"));
    }

    #[test]
    fn test_info_from_command_manager() {
        let command_manager = super::ForgeCommandManager::default();
        let info = super::Info::from(&command_manager);
        let display = info.to_string();

        // Verify compile-time detection works correctly
        #[cfg(target_os = "macos")]
        {
            assert!(display.contains("<opt+enter>"));
            assert!(!display.contains("<alt+enter>"));
        }

        #[cfg(not(target_os = "macos"))]
        {
            assert!(display.contains("<alt+enter>"));
            assert!(!display.contains("<opt+enter>"));
        }

        // Should contain standard sections
        assert!(display.contains("COMMANDS"));
        assert!(display.contains("KEYBOARD SHORTCUTS"));
        assert!(display.contains("<ctrl+c>"));
        assert!(display.contains("<ctrl+d>"));
    }

    #[test]
    fn test_metrics_info_filters_zero_changes() {
        use forge_api::Metrics;
        use forge_domain::{FileOperation, ToolKind};

        let fixture = Metrics::default()
            .started_at(chrono::Utc::now())
            .insert(
                "src/main.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(12u64)
                    .lines_removed(3u64),
            )
            .insert(
                "src/no_changes.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(0u64)
                    .lines_removed(0u64),
            )
            .insert(
                "src/agent/mod.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(8u64)
                    .lines_removed(2u64),
            );

        let actual = super::Info::from(&fixture);
        let expected_display = actual.to_string();

        // Verify it contains the task completed section
        assert!(expected_display.contains("TASK COMPLETED"));

        // Verify it contains files with changes as keys
        assert!(expected_display.contains("main.rs"));
        assert!(expected_display.contains("−3 +12"));
        assert!(expected_display.contains("mod.rs"));
        assert!(expected_display.contains("−2 +8"));

        // Verify it does NOT contain the file with zero changes
        assert!(!expected_display.contains("no_changes.rs"));
    }

    #[test]
    fn test_metrics_info_all_zero_changes_shows_no_changes() {
        use forge_api::Metrics;
        use forge_domain::{FileOperation, ToolKind};

        let fixture = Metrics::default()
            .started_at(chrono::Utc::now())
            .insert(
                "src/file1.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(0u64)
                    .lines_removed(0u64),
            )
            .insert(
                "src/file2.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(0u64)
                    .lines_removed(0u64),
            );

        let actual = super::Info::from(&fixture);
        let expected_display = actual.to_string();

        // Verify it shows "No Changes Produced" when all files have zero changes
        assert!(expected_display.contains("[No Changes Produced]"));
        assert!(!expected_display.contains("file1.rs"));
        assert!(!expected_display.contains("file2.rs"));
    }
}
