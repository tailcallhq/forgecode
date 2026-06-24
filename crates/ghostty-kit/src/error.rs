//! Typed errors produced by the Ghostty config parser.
//!
//! Errors always carry a file path and a 1-based line number so that
//! downstream tooling (shell-plugin, `forge ghostty config` CLI) can
//! surface actionable diagnostics back to the user.

use std::fmt;
use std::path::PathBuf;

/// Errors that can occur while parsing or resolving a Ghostty config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// The input could not be read from disk.
    Io {
        path: PathBuf,
        message: String,
    },
    /// A line did not match the `key = value` shape, the `[section]`
    /// shape, a comment, or a blank line.
    MalformedLine {
        path: PathBuf,
        line: usize,
        content: String,
    },
    /// A `[section]` header was opened but never closed on the same line.
    UnterminatedSection {
        path: PathBuf,
        line: usize,
        content: String,
    },
    /// A `keybind` or `command` directive was missing its right-hand side.
    MissingValue {
        path: PathBuf,
        line: usize,
        key: String,
    },
    /// A color literal was not in `#RRGGBB` or `#RRGGBBAA` form.
    InvalidColor {
        path: PathBuf,
        line: usize,
        value: String,
    },
    /// A `config-file = ...` directive pointed at a file that does not exist.
    MissingInclude {
        path: PathBuf,
        line: usize,
        included: PathBuf,
    },
    /// A `config-file = ...` chain created a cycle.
    RecursiveInclude {
        path: PathBuf,
        cycle: Vec<PathBuf>,
    },
    /// A `$name` or `${name}` variable was used but not provided.
    UndefinedVariable {
        path: PathBuf,
        line: usize,
        name: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io { path, message } => {
                write!(f, "I/O error reading {}: {}", path.display(), message)
            }
            ConfigError::MalformedLine {
                path,
                line,
                content,
            } => write!(
                f,
                "malformed line in {}:{}: `{}`",
                path.display(),
                line,
                content
            ),
            ConfigError::UnterminatedSection {
                path,
                line,
                content,
            } => write!(
                f,
                "unterminated section header in {}:{}: `{}`",
                path.display(),
                line,
                content
            ),
            ConfigError::MissingValue { path, line, key } => write!(
                f,
                "missing value for key `{}` in {}:{}",
                key,
                path.display(),
                line
            ),
            ConfigError::InvalidColor { path, line, value } => write!(
                f,
                "invalid color literal `{}` in {}:{} (expected #RRGGBB or #RRGGBBAA)",
                value,
                path.display(),
                line
            ),
            ConfigError::MissingInclude {
                path,
                line,
                included,
            } => write!(
                f,
                "include file not found: `{}` (referenced from {}:{})",
                included.display(),
                path.display(),
                line
            ),
            ConfigError::RecursiveInclude { path, cycle } => {
                write!(f, "recursive include detected through {}: ", path.display())?;
                for (i, p) in cycle.iter().enumerate() {
                    if i > 0 {
                        write!(f, " -> ")?;
                    }
                    write!(f, "{}", p.display())?;
                }
                Ok(())
            }
            ConfigError::UndefinedVariable { path, line, name } => write!(
                f,
                "undefined variable `${}` in {}:{}",
                name,
                path.display(),
                line
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Convenience alias used by the parser API.
pub type Result<T> = std::result::Result<T, ConfigError>;
