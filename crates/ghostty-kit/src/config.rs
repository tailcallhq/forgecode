//! Ghostty INI-style config parser.
//!
//! The parser is line-oriented and handles the subset of Ghostty's
//! config syntax that the forgecode integration needs. See [`parse`]
//! for the entry point and the crate-level docs for a list of
//! supported features.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ConfigError, Result};
use crate::value::{infer_value, substitute_value};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A parsed Ghostty configuration file.
///
/// `entries` preserves the order in which directives appeared in the
/// source. `includes` is a convenience copy of every `config-file`
/// directive (also reachable via `Include` entries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhosttyConfig {
    pub entries: Vec<ConfigEntry>,
    pub includes: Vec<PathBuf>,
    pub source: PathBuf,
}

/// One entry inside a Ghostty config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigEntry {
    KeyValue {
        key: String,
        value: ConfigValue,
        section: Option<String>,
        line: usize,
    },
    Include(PathBuf),
    Section(String, usize),
}

/// A typed configuration value.
///
/// Ghostty is untyped at the wire level — the parser infers a type
/// from the literal shape. See [`crate::value::infer_value`] for the
/// rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValue {
    String(String),
    Bool(bool),
    Integer(i64),
    /// RGBA packed as `0xRRGGBBAA`. Missing alpha is `0xFF`.
    Color(u32),
    List(Vec<String>),
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a Ghostty config from an in-memory string.
///
/// `file` is the logical path the source came from; it is used only
/// for diagnostics (errors and the [`GhosttyConfig::source`] field).
pub fn parse(source: &str, file: PathBuf) -> Result<GhosttyConfig> {
    let mut parser = Parser::new(file);
    parser.run(source)?;
    Ok(GhosttyConfig {
        entries: parser.entries,
        includes: parser.includes,
        source: parser.file,
    })
}

/// Parse a Ghostty config from disk.
pub fn parse_file(path: &Path) -> Result<GhosttyConfig> {
    let source = fs::read_to_string(path).map_err(|e| ConfigError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    parse(&source, path.to_path_buf())
}

/// Resolve a config and all of its `config-file` includes into a flat
/// list, root first.
///
/// `base` is the directory relative to which bare (non-absolute)
/// `config-file` paths are resolved. The returned vector always starts
/// with the root [`GhosttyConfig`] and is followed by includes in the
/// order they were declared (depth-first). Cycles produce
/// [`ConfigError::RecursiveInclude`].
pub fn resolve_includes(
    config: &GhosttyConfig,
    base: &Path,
) -> Result<Vec<GhosttyConfig>> {
    let mut out = Vec::new();
    let mut visiting: Vec<PathBuf> = Vec::new();

    out.push(config.clone());
    visit(config, base, &mut out, &mut visiting)?;
    Ok(out)
}

fn visit(
    config: &GhosttyConfig,
    base: &Path,
    out: &mut Vec<GhosttyConfig>,
    visiting: &mut Vec<PathBuf>,
) -> Result<()> {
    visiting.push(config.source.clone());
    for entry in &config.entries {
        if let ConfigEntry::Include(raw) = entry {
            let path = resolve_include_path(raw, base);
            if let Some(pos) = visiting.iter().position(|p| p == &path) {
                let mut cycle: Vec<PathBuf> = visiting[pos..].to_vec();
                cycle.push(path.clone());
                return Err(ConfigError::RecursiveInclude {
                    path: config.source.clone(),
                    cycle,
                });
            }
            if !path.exists() {
                return Err(ConfigError::MissingInclude {
                    path: config.source.clone(),
                    line: 0,
                    included: path,
                });
            }
            let nested = parse_file(&path)?;
            out.push(nested.clone());
            visit(&nested, &parent_dir(&path), out, visiting)?;
        }
    }
    visiting.pop();
    Ok(())
}

/// Substitute `$name` and `${name}` placeholders in every
/// `String`/`List` value of the given config.
///
/// Undefined variables are left as their literal form rather than
/// raising an error — the downstream render step can choose how to
/// surface them.
pub fn substitute_variables(
    config: &GhosttyConfig,
    vars: &HashMap<String, String>,
) -> GhosttyConfig {
    let entries = config
        .entries
        .iter()
        .map(|entry| match entry {
            ConfigEntry::KeyValue {
                key,
                value,
                section,
                line,
            } => ConfigEntry::KeyValue {
                key: key.clone(),
                value: substitute_value(value, vars),
                section: section.clone(),
                line: *line,
            },
            other => other.clone(),
        })
        .collect();
    GhosttyConfig {
        entries,
        includes: config.includes.clone(),
        source: config.source.clone(),
    }
}

/// Return the first [`ConfigValue`] whose key matches `key` exactly.
///
/// The search is order-preserving: the first `KeyValue` entry with a
/// matching key wins, regardless of section.
pub fn get<'a>(config: &'a GhosttyConfig, key: &str) -> Option<&'a ConfigValue> {
    for entry in &config.entries {
        match entry {
            ConfigEntry::KeyValue {
                key: k, value, ..
            } if k == key => return Some(value),
            _ => {}
        }
    }
    None
}

/// Return every [`ConfigEntry::KeyValue`] whose `section` field equals
/// `section`.
pub fn get_section<'a>(
    config: &'a GhosttyConfig,
    section: &str,
) -> Vec<&'a ConfigEntry> {
    config
        .entries
        .iter()
        .filter(|e| matches!(e, ConfigEntry::KeyValue { section: Some(s), .. } if s == section))
        .collect()
}

// ---------------------------------------------------------------------------
// JSON serializer for golden tests
// ---------------------------------------------------------------------------
//
// Re-exported from `serialize` so integration tests can grab it via
// `ghostty_kit::to_json`. The serializer lives in its own module to
// keep this file under the project's line-count budget.

#[doc(hidden)]
pub use crate::serialize::to_json;

// ---------------------------------------------------------------------------
// Line-oriented parser
// ---------------------------------------------------------------------------

struct Parser {
    file: PathBuf,
    entries: Vec<ConfigEntry>,
    includes: Vec<PathBuf>,
    current_section: Option<String>,
}

impl Parser {
    fn new(file: PathBuf) -> Self {
        Self {
            file,
            entries: Vec::new(),
            includes: Vec::new(),
            current_section: None,
        }
    }

    fn run(&mut self, source: &str) -> Result<()> {
        let mut buffer: Option<String> = None;
        let mut buffer_start_line: usize = 0;
        let mut current_line: usize = 0;

        for raw_line in source.lines() {
            current_line += 1;
            // A line that ends with `\` (after trimming) extends the
            // previous logical line.
            let trimmed_end = raw_line.trim_end();
            if let Some(stripped) = trimmed_end.strip_suffix('\\') {
                if buffer.is_none() {
                    buffer_start_line = current_line;
                    buffer = Some(String::new());
                }
                buffer
                    .as_mut()
                    .unwrap()
                    .push_str(stripped.trim_end());
                buffer.as_mut().unwrap().push('\n');
                continue;
            }

            let logical = if let Some(mut buf) = buffer.take() {
                buf.push_str(raw_line);
                buf
            } else {
                raw_line.to_string()
            };
            let logical_line_no = if buffer_start_line != 0 {
                buffer_start_line
            } else {
                current_line
            };

            self.process_line(&logical, logical_line_no)?;
        }

        // A trailing `\` with no following line still forms a logical
        // line that must be processed.
        if let Some(buf) = buffer.take() {
            self.process_line(&buf, buffer_start_line)?;
        }
        Ok(())
    }

    fn process_line(&mut self, line: &str, line_no: usize) -> Result<()> {
        // A `#` starts a comment only when:
        //   (a) it's the first non-whitespace character on the line, OR
        //   (b) it's preceded by whitespace AND followed by whitespace
        //       or end-of-line.
        // This preserves color literals like `#RRGGBB` that appear
        // immediately after the `=` separator (no whitespace after `#`).
        if is_full_line_comment(line) {
            return Ok(());
        }
        let stripped = strip_inline_comment(line);
        let trimmed = stripped.trim();
        if trimmed.is_empty() {
            return Ok(());
        }

        // Section header: `[name]`
        if let Some(rest) = trimmed.strip_prefix('[') {
            match rest.strip_suffix(']') {
                Some(name) => {
                    let name = name.trim().to_string();
                    if name.is_empty() {
                        return Err(ConfigError::UnterminatedSection {
                            path: self.file.clone(),
                            line: line_no,
                            content: line.to_string(),
                        });
                    }
                    self.current_section = Some(name.clone());
                    self.entries.push(ConfigEntry::Section(name, line_no));
                    return Ok(());
                }
                None => {
                    return Err(ConfigError::UnterminatedSection {
                        path: self.file.clone(),
                        line: line_no,
                        content: line.to_string(),
                    });
                }
            }
        }

        // Key/value pair: `key = value`
        let (key, value) = match split_key_value(trimmed) {
            Some(kv) => kv,
            None => {
                return Err(ConfigError::MalformedLine {
                    path: self.file.clone(),
                    line: line_no,
                    content: line.to_string(),
                });
            }
        };

        // Special-case `config-file`: emit an Include entry, do not
        // store as a key/value.
        if key == "config-file" {
            let path = PathBuf::from(value);
            self.includes.push(path.clone());
            self.entries.push(ConfigEntry::Include(path));
            return Ok(());
        }

        if value.is_empty() {
            return Err(ConfigError::MissingValue {
                path: self.file.clone(),
                line: line_no,
                key: key.to_string(),
            });
        }

        let parsed = infer_value(key, value);
        self.entries.push(ConfigEntry::KeyValue {
            key: key.to_string(),
            value: parsed,
            section: self.current_section.clone(),
            line: line_no,
        });
        Ok(())
    }
}

fn is_full_line_comment(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

fn strip_inline_comment(line: &str) -> &str {
    // Look for a `#` that is *clearly* an inline comment marker:
    //   - preceded by whitespace,
    //   - followed by whitespace or end-of-line.
    // A `#` at column 0 (handled by `is_full_line_comment`) or one
    // glued to a non-space token (e.g. the start of a hex color) is
    // left intact.
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'#' {
            continue;
        }
        let preceded = i == 0 || matches!(bytes[i - 1], b' ' | b'\t');
        let followed = i + 1 == bytes.len() || matches!(bytes[i + 1], b' ' | b'\t');
        if preceded && followed {
            return &line[..i];
        }
    }
    line
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let key = line[..eq].trim();
    let value = line[eq + 1..].trim();
    Some((key, value))
}

fn resolve_include_path(raw: &Path, base: &Path) -> PathBuf {
    if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base.join(raw)
    }
}

fn parent_dir(path: &Path) -> PathBuf {
    path.parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}
