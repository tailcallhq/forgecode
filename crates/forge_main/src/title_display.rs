use std::fmt;

use chrono::Local;
use colored::Colorize;
use forge_domain::{Category, TitleFormat};

/// Implementation of Display for TitleFormat in the presentation layer
pub struct TitleDisplay {
    inner: TitleFormat,
    with_colors: bool,
}

impl TitleDisplay {
    pub fn new(title: TitleFormat) -> Self {
        Self { inner: title, with_colors: true }
    }

    pub fn with_colors(mut self, with_colors: bool) -> Self {
        self.with_colors = with_colors;
        self
    }

    fn format_with_colors(&self) -> String {
        let mut buf = String::new();

        let icon = match self.inner.category {
            Category::Action => "●".yellow(),
            Category::Info => "●".white(),
            Category::Debug => "●".cyan(),
            Category::Error => "●".red(),
            Category::Completion => "●".yellow(),
            Category::Warning => "⚠️".bright_yellow(),
        };

        buf.push_str(format!("{icon} ").as_str());

        let local_time: chrono::DateTime<Local> = self.inner.timestamp.into();
        let timestamp_str = format!("[{}] ", local_time.format("%H:%M:%S"));
        buf.push_str(timestamp_str.dimmed().to_string().as_str());

        let title = match self.inner.category {
            Category::Action => self.inner.title.white(),
            Category::Info => self.inner.title.white(),
            Category::Debug => self.inner.title.dimmed(),
            Category::Error => format!("{} {}", "ERROR:".bold(), self.inner.title).red(),
            Category::Completion => self.inner.title.white().bold(),
            Category::Warning => {
                format!("{} {}", "WARNING:".bold(), self.inner.title).bright_yellow()
            }
        };

        buf.push_str(title.to_string().as_str());

        if let Some(ref sub_title) = self.inner.sub_title {
            buf.push_str(&format!(" {}", sub_title.dimmed()).to_string());
        }

        buf
    }

    fn format_plain(&self) -> String {
        let mut buf = String::new();

        buf.push_str("● ");

        let local_time: chrono::DateTime<Local> = self.inner.timestamp.into();
        let timestamp_str = format!("[{}] ", local_time.format("%H:%M:%S"));
        buf.push_str(&timestamp_str);

        buf.push_str(&self.inner.title);

        if let Some(ref sub_title) = self.inner.sub_title {
            buf.push_str(&format!(" {sub_title}"));
        }

        buf
    }
}

impl fmt::Display for TitleDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.with_colors {
            write!(f, "{}", self.format_with_colors())
        } else {
            write!(f, "{}", self.format_plain())
        }
    }
}

/// Extension trait to easily convert TitleFormat to displayable form
pub trait TitleDisplayExt {
    fn display(self) -> TitleDisplay;
    fn display_with_colors(self, with_colors: bool) -> TitleDisplay;
}

impl TitleDisplayExt for TitleFormat {
    fn display(self) -> TitleDisplay {
        TitleDisplay::new(self)
    }

    fn display_with_colors(self, with_colors: bool) -> TitleDisplay {
        TitleDisplay::new(self).with_colors(with_colors)
    }
}
