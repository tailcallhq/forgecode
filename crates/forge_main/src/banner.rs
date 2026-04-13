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
/// * `cli_mode` - If true, shows CLI-relevant commands with `:` prefix. If
///   false, shows all interactive commands with `/` prefix.
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
            ("New conversation:", "/new"),
            ("Get started:", "/info, /usage, /help, /conversation"),
            ("Switch model:", "/model"),
            ("Switch agent:", "/forge or /muse or /agent"),
            ("Update:", "/update"),
            ("Quit:", "/exit or <CTRL+D>"),
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

    // Encourage zsh integration after the banner
    if !cli_mode {
        display_zsh_encouragement();
    }

    Ok(())
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
