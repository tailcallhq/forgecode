use std::{fmt, io};

use colored::Colorize;
use forge_tracker::VERSION;

const BANNER: &str = include_str!("banner");

/// Renders messages into a styled box with border characters.
struct DisplayBox {
    messages: Vec<String>,
}

impl DisplayBox {
    /// Creates a new Box with the given messages.
    fn new(messages: Vec<String>) -> Self {
        Self { messages }
    }
}

impl fmt::Display for DisplayBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let visible_len = |s: &str| console::measure_text_width(s);
        let width: usize = self
            .messages
            .iter()
            .map(|s| visible_len(s))
            .max()
            .unwrap_or(0)
            + 4;
        let top = format!("┌{}┐", "─".repeat(width.saturating_sub(2)));
        let bottom = format!("└{}┘", "─".repeat(width.saturating_sub(2)));
        let fmt_line = |s: &str| {
            let padding = width.saturating_sub(4).saturating_sub(visible_len(s));
            format!("│ {}{} │", s, " ".repeat(padding))
        };

        writeln!(f, "{}", top)?;
        for msg in &self.messages {
            writeln!(f, "{}", fmt_line(msg))?;
        }
        write!(f, "{}", bottom)
    }
}

/// Displays the banner with version and command tips.
///
/// # Arguments
///
/// * `cli_mode` - If true, shows CLI-relevant commands. Both interactive and
///   CLI modes use `:` as the canonical command prefix.
///
/// # Environment Variables
///
/// * `FORGE_BANNER` - Optional custom banner text to display instead of the
///   default
pub fn display(cli_mode: bool) -> io::Result<()> {
    // Check for custom banner via environment variable
    let mut banner = std::env::var("FORGE_BANNER")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| BANNER.to_string());

    // Always show version
    let version_label = ("Version:", VERSION);

    // Build tips based on mode
    let tips: Vec<(&str, &str)> = if cli_mode {
        // CLI mode: only show relevant commands
        vec![
            ("New conversation:", ":new"),
            ("Get started:", ":info, :conversation"),
            ("Switch model:", ":model"),
            ("Switch provider:", ":provider"),
            ("Switch agent:", ":<agent_name> e.g. :forge or :muse"),
        ]
    } else {
        // Interactive mode: show all commands
        vec![
            ("New conversation:", ":new"),
            ("Get started:", ":info, :usage, :help, :conversation"),
            ("Switch model:", ":model"),
            ("Switch agent:", ":forge or :muse or :agent"),
            ("Update:", ":update"),
            ("Quit:", ":exit or <CTRL+D>"),
        ]
    };

    // Build labels array with version and tips
    let labels: Vec<(&str, &str)> = std::iter::once(version_label).chain(tips).collect();

    // Calculate the width of the longest label key for alignment
    let max_width = labels.iter().map(|(key, _)| key.len()).max().unwrap_or(0);

    // Add all lines with right-aligned label keys and their values
    for (key, value) in &labels {
        banner.push_str(
            format!(
                "\n{}{}",
                format!("{key:>max_width$} ").dimmed(),
                value.cyan()
            )
            .as_str(),
        );
    }

    println!("{banner}\n");

    if !cli_mode && !is_zsh_plugin_loaded() {
        display_zsh_encouragement();
    }

    Ok(())
}

/// Returns `true` when the zsh plugin exported `_FORGE_PLUGIN_LOADED` into the
/// environment inherited by this forge process.
fn is_zsh_plugin_loaded() -> bool {
    std::env::var("_FORGE_PLUGIN_LOADED")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Encourages users to use the zsh plugin for a better experience.
fn display_zsh_encouragement() {
    let tip = DisplayBox::new(vec![
        format!(
            "{} {}",
            "TIP:".bold().yellow(),
            "For the best experience, use our zsh plugin!".bold()
        ),
        format!(
            "{} {} {}",
            "·".dimmed(),
            "Set up forge via our zsh plugin:".dimmed(),
            "forge zsh setup".bold().green(),
        ),
        format!(
            "{} {} {}",
            "·".dimmed(),
            "Learn more:".dimmed(),
            "https://forgecode.dev/docs/zsh-support".cyan()
        ),
    ]);
    println!("{}", tip);
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::OsString;
    use std::sync::{LazyLock, Mutex, MutexGuard};

    use super::*;

    static ENV_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct EnvGuard {
        key: &'static str,
        original_value: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|error| error.into_inner());
            let original_value = env::var_os(key);
            unsafe {
                env::set_var(key, value);
            }
            Self { key, original_value, _lock: lock }
        }

        fn unset(key: &'static str) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|error| error.into_inner());
            let original_value = env::var_os(key);
            unsafe {
                env::remove_var(key);
            }
            Self { key, original_value, _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original_value {
                Some(value) => unsafe {
                    env::set_var(self.key, value);
                },
                None => unsafe {
                    env::remove_var(self.key);
                },
            }
        }
    }

    #[test]
    fn test_is_zsh_plugin_loaded_when_env_var_set() {
        let _guard = EnvGuard::set("_FORGE_PLUGIN_LOADED", "1700000000");
        assert!(is_zsh_plugin_loaded());
    }

    #[test]
    fn test_is_zsh_plugin_loaded_when_env_var_missing() {
        let _guard = EnvGuard::unset("_FORGE_PLUGIN_LOADED");
        assert!(!is_zsh_plugin_loaded());
    }
}
