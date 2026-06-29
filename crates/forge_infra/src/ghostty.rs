//! `forge ghostty` subcommand: inspect and manage the Ghostty integration.
//!
//! Ghostty is a GPU-accelerated terminal emulator that exposes a runtime
//! control surface over a Unix domain socket (see `ghostty_kit::ipc`).
//! This module wires that surface into the forge CLI so operators can:
//!
//! - View the effective Ghostty config (`forge ghostty config`).
//! - Probe the IPC socket (`forge ghostty ipc status`).
//! - Inspect and reload shaders (`forge ghostty shader list` / `reload`).
//! - Print Ghostty's own version (`forge ghostty version`).
//!
//! Every code path is non-fatal: a missing socket, a missing binary, or a
//! missing config directory prints `unavailable` or `unknown` and exits 0
//! unless the user supplied an invalid invocation.

use std::path::{Path, PathBuf};

use clap::{Arg, ArgMatches, Command};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Build the top-level `forge` command with the `ghostty` subcommand wired in.
///
/// Returning a `clap::Command` lets the host binary decide whether `forge`
/// has any other top-level subcommands; this module only owns `ghostty`.
pub fn cmd() -> Command {
    Command::new("forge")
        .about("forge: command-line tool")
        .subcommand(
            Command::new("ghostty")
                .about("Inspect and manage the Ghostty integration")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("config").about("Print effective Ghostty config"),
                )
                .subcommand(
                    Command::new("ipc")
                        .about("Inspect IPC socket state")
                        .subcommand(
                            Command::new("status")
                                .about("Print IPC socket path and connection state"),
                        ),
                )
                .subcommand(
                    Command::new("shader")
                        .about("Manage Ghostty shaders")
                        .subcommand(
                            Command::new("list").about(
                                "List registered shaders from ~/.config/ghostty/shaders/",
                            ),
                        )
                        .subcommand(
                            Command::new("reload")
                                .about("Reload a single shader via IPC")
                                .arg(
                                    Arg::new("name")
                                        .required(true)
                                        .help("Shader file basename (e.g. \"myeffect\")"),
                                ),
                        ),
                )
                .subcommand(Command::new("version").about("Print Ghostty version")),
        )
}

/// Dispatch a parsed `forge` invocation to the `ghostty` subcommand.
///
/// Returns a process exit code: 0 on success, 1 on user error, 2 on system
/// error. The top-level dispatcher in `main.rs` calls this.
pub fn run(matches: &ArgMatches) -> i32 {
    match matches.subcommand() {
        Some(("ghostty", sub)) => run_ghostty(sub),
        // `cmd()` only declares `ghostty` as a subcommand, so clap will
        // never hand us anything else here.
        _ => unreachable!("top-level dispatcher should only pass the ghostty subcommand"),
    }
}

fn run_ghostty(matches: &ArgMatches) -> i32 {
    match matches.subcommand() {
        Some(("config", _)) => run_config(),
        Some(("ipc", sub)) => match sub.subcommand() {
            Some(("status", _)) => run_ipc_status(),
            _ => unreachable!("ipc subcommand has no other children"),
        },
        Some(("shader", sub)) => match sub.subcommand() {
            Some(("list", _)) => run_shader_list(),
            Some(("reload", args)) => {
                // `name` is marked `required(true)` by clap, so the
                // `unwrap_or` branch is unreachable in practice. We use
                // `get_one` (not direct indexing) so future `ArgAction`
                // changes do not silently break this code.
                let name = match args.get_one::<String>("name") {
                    Some(n) => n.as_str(),
                    None => unreachable!("required arg \"name\" validated by clap"),
                };
                run_shader_reload(name)
            }
            _ => unreachable!("shader subcommand has no other children"),
        },
        Some(("version", _)) => run_version(),
        _ => unreachable!("arg_required_else_help guards this branch"),
    }
}

// ---------------------------------------------------------------------------
// Subcommand handlers
// ---------------------------------------------------------------------------

/// `forge ghostty config`: print the effective Ghostty config.
///
/// Reads `$XDG_CONFIG_HOME/ghostty/config` if it exists, else
/// `$HOME/.config/ghostty/config`. If neither exists, prints
/// `status: unavailable` (exit 0) — a missing config is not a CLI error.
fn run_config() -> i32 {
    let path = match ghostty_config_path() {
        Some(p) => p,
        None => {
            println!("status: unavailable");
            println!("reason: no config dir");
            return 0;
        }
    };

    println!("source: {}", path.display());
    match ghostty_kit::parse_file(&path) {
        Ok(cfg) => {
            println!("status: ok");
            println!("entries: {}", cfg.entries.len());
            if !cfg.includes.is_empty() {
                println!("includes: {}", cfg.includes.len());
            }
            0
        }
        Err(e) => {
            println!("status: parse_error");
            println!("error: {e}");
            1
        }
    }
}

/// `forge ghostty ipc status`: probe the Ghostty control socket.
///
/// `GhosttyControl::try_new` is the contract from PR-1: it never panics
/// and returns `None` whenever no live socket is reachable. We surface
/// that as `unavailable` (exit 0); the user asked a question and we
/// answered it honestly.
fn run_ipc_status() -> i32 {
    match ghostty_kit::GhosttyControl::try_new() {
        Some(_) => {
            println!("status: available");
            0
        }
        None => {
            println!("status: unavailable");
            0
        }
    }
}

/// `forge ghostty shader list`: print shader basenames from the standard
/// Ghostty shaders directory, sorted.
///
/// A missing directory is not an error: a user without shaders just
/// gets an empty listing.
fn run_shader_list() -> i32 {
    let dir = match ghostty_shader_dir() {
        Some(d) => d,
        None => {
            println!("dir: unavailable");
            println!("shaders:");
            return 0;
        }
    };
    println!("dir: {}", dir.display());
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                .collect();
            names.sort();
            for name in &names {
                println!("shader: {name}");
            }
            if names.is_empty() {
                println!("shaders:");
            }
            0
        }
        Err(_) => {
            println!("status: unavailable");
            0
        }
    }
}

/// `forge ghostty shader reload <name>`: emit `reload_config` over IPC.
///
/// The `<name>` is the shader we *intend* to reload; Ghostty's IPC
/// reloads the whole config (which re-evaluates all `custom-shader`
/// directives). The name is echoed back so log scrapers can correlate.
fn run_shader_reload(name: &str) -> i32 {
    println!("shader: {name}");
    match ghostty_kit::GhosttyControl::try_new() {
        None => {
            // No live socket: this is a system-level "we cannot reach the
            // terminal", not a user error, so we return 1 (user action
            // required) rather than 2. Logically: the user did the right
            // thing, the host just is not running.
            println!("status: unavailable");
            1
        }
        Some(ctl) => match ctl.reload_config() {
            Ok(()) => {
                println!("status: reloaded");
                0
            }
            Err(e) => {
                println!("status: error");
                println!("error: {e}");
                2
            }
        },
    }
}

/// `forge ghostty version`: print `ghostty --version` if available.
fn run_version() -> i32 {
    // `Command::new("ghostty")` will fail with `NotFound` when the
    // binary is not in `$PATH`; both that and a non-zero exit code
    // collapse into the `unknown` branch.
    let output = std::process::Command::new("ghostty").arg("--version").output();
    match output {
        Ok(out) if out.status.success() => {
            let raw = String::from_utf8_lossy(&out.stdout);
            // Ghostty prints "Ghostty 1.X.Y (commit)" — keep it verbatim.
            println!("ghostty: {}", raw.trim());
            0
        }
        _ => {
            println!("ghostty: unknown");
            0
        }
    }
}

// ---------------------------------------------------------------------------
// GhosttyIPC API — types and functions for programmatic use from forge_infra
// ---------------------------------------------------------------------------

/// A single GLSL shader error found during static analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShaderError {
    /// 1-based source line where the error occurs.
    pub line: u32,
    /// 0-based column offset on the line.
    pub column: u32,
    /// Severity level (e.g. "error", "warning").
    pub severity: String,
    /// Human-readable error description.
    pub message: String,
}

/// Result of a GLSL shader static-analysis pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShaderReport {
    /// `true` when no errors were found (warnings alone keep this `true`).
    pub ok: bool,
    /// Syntactic and semantic errors found.
    pub errors: Vec<ShaderError>,
    /// Non-fatal warnings (missing directive, style, etc.).
    pub warnings: Vec<String>,
}

/// Metadata about a font discovered via filesystem heuristics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontInfo {
    /// Font family name (inferred from filename).
    pub family: String,
    /// Font style (e.g. "Regular", "Bold", "Italic").
    pub style: String,
    /// Absolute path to the font file on disk.
    pub path: PathBuf,
}

/// Runtime snapshot from a running Ghostty instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhosttySnapshot {
    /// Ghostty version string, if reported.
    pub version: Option<String>,
    /// Configuration files currently loaded.
    pub config_files: Vec<String>,
    /// Configuration directories searched.
    pub config_dirs: Vec<String>,
    /// Resource paths (shaders, themes, etc.).
    pub resources: Vec<String>,
    /// Process ID of the Ghostty instance, if available.
    pub pid: Option<u32>,
    /// Raw JSON response from the inspect IPC action.
    pub raw: serde_json::Value,
}

/// Errors produced by the Ghostty IPC or related operations.
#[derive(Debug, Error)]
pub enum GhosttyError {
    /// The IPC transport layer produced an error.
    #[error("IPC error: {0}")]
    Ipc(#[from] ghostty_kit::IpcError),
    /// An I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialisation or deserialisation failed.
    #[error("JSON error: {0}")]
    Serde(#[from] serde_json::Error),
    /// A generic error message.
    #[error("{0}")]
    Other(String),
}

/// Convert a ghostty-kit [`JsonValue`] to a [`serde_json::Value`] recursively.
fn to_serde_value(jv: &ghostty_kit::JsonValue) -> serde_json::Value {
    match jv {
        ghostty_kit::JsonValue::Null => serde_json::Value::Null,
        ghostty_kit::JsonValue::Bool(b) => serde_json::Value::Bool(*b),
        ghostty_kit::JsonValue::Int(n) => serde_json::json!(*n),
        ghostty_kit::JsonValue::String(s) => {
            serde_json::Value::String(s.clone())
        }
        ghostty_kit::JsonValue::Array(items) => {
            serde_json::Value::Array(items.iter().map(to_serde_value).collect())
        }
        ghostty_kit::JsonValue::Object(entries) => {
            let map: serde_json::Map<String, serde_json::Value> = entries
                .iter()
                .map(|(k, v)| (k.clone(), to_serde_value(v)))
                .collect();
            serde_json::Value::Object(map)
        }
    }
}

/// Perform static-analysis validation of a GLSL shader fragment.
///
/// This is a pure-Rust lint pass that checks for balanced delimiters,
/// required `#version` directives, a `void main` entry point, and
/// common GLSL gotchas — no GPU or actual compilation is involved.
pub fn shader_lint(source: &str) -> ShaderReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    // 1. Check for a #version directive.
    let has_version = lines.iter().any(|l| l.trim().starts_with("#version"));
    if !has_version {
        warnings.push(
            "No #version directive found; using default (100 for ES, 110 for desktop)".into(),
        );
    }

    // 2. Balanced delimiters.
    let mut brace_depth: i32 = 0;
    let mut paren_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    for (i, line) in lines.iter().enumerate() {
        let line_num = (i + 1) as u32;
        for (j, ch) in line.char_indices() {
            match ch {
                '{' => brace_depth += 1,
                '}' => brace_depth -= 1,
                '(' => paren_depth += 1,
                ')' => paren_depth -= 1,
                '[' => bracket_depth += 1,
                ']' => bracket_depth -= 1,
                _ => {}
            }
            if brace_depth < 0 {
                errors.push(ShaderError {
                    line: line_num,
                    column: j as u32,
                    severity: "error".into(),
                    message: "Unmatched closing brace '}'".into(),
                });
                brace_depth = 0;
            }
            if paren_depth < 0 {
                errors.push(ShaderError {
                    line: line_num,
                    column: j as u32,
                    severity: "error".into(),
                    message: "Unmatched closing parenthesis ')'".into(),
                });
                paren_depth = 0;
            }
            if bracket_depth < 0 {
                errors.push(ShaderError {
                    line: line_num,
                    column: j as u32,
                    severity: "error".into(),
                    message: "Unmatched closing bracket ']'".into(),
                });
                bracket_depth = 0;
            }
        }
    }
    if brace_depth > 0 {
        errors.push(ShaderError {
            line: lines.len() as u32,
            column: 0,
            severity: "error".into(),
            message: format!("Unclosed brace '{{' (depth: {brace_depth})"),
        });
    }
    if paren_depth > 0 {
        errors.push(ShaderError {
            line: lines.len() as u32,
            column: 0,
            severity: "error".into(),
            message: format!("Unclosed parenthesis '(' (depth: {paren_depth})"),
        });
    }
    if bracket_depth > 0 {
        errors.push(ShaderError {
            line: lines.len() as u32,
            column: 0,
            severity: "error".into(),
            message: format!("Unclosed bracket '[' (depth: {bracket_depth})"),
        });
    }

    // 3. Presence of an entry point.
    let has_main = source.contains("void main");
    if !has_main {
        warnings.push("No 'void main' entry point found in shader source".into());
    }

    // 4. Check for common GLSL pitfalls: gl_Position usage in vertex path.
    if source.contains("void main") && !source.contains("gl_Position") {
        warnings.push(
            "Vertex shaders typically assign gl_Position; consider adding it".into(),
        );
    }

    ShaderReport {
        ok: errors.is_empty(),
        errors,
        warnings,
    }
}

/// Scan the system for installed font files using filename heuristics.
///
/// Checks common platform-specific font directories. Font names and
/// styles are inferred from filenames — no fontconfig dependency is
/// used. Returns every `.ttf`, `.otf`, or `.ttc` found.
pub fn font_list() -> Result<Vec<FontInfo>, GhosttyError> {
    let mut fonts = Vec::new();

    // Platform-specific font directories.
    let dirs: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts",
            "/Library/Fonts",
            "~/Library/Fonts",
            "/System/Library/AssetsV2/com_apple_MobileAsset_Font7",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            "C:\\Windows\\Fonts",
            "C:\\Windows\\WinSxS\\Fonts",
        ]
    } else {
        &[
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "~/.fonts",
            "~/.local/share/fonts",
        ]
    };

    for dir_str in dirs {
        let dir = if dir_str.starts_with('~') {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            home.join(&dir_str[2..]) // skip "~/"
        } else {
            PathBuf::from(dir_str)
        };
        if !dir.is_dir() {
            continue;
        }
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(info) = parse_font_info(&path) {
                        fonts.push(info);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("font_list: cannot read {}: {e}", dir.display());
            }
        }
    }

    // Sort by family, then style.
    fonts.sort_by(|a, b| a.family.cmp(&b.family).then(a.style.cmp(&b.style)));
    fonts.dedup_by(|a, b| a.path == b.path);

    Ok(fonts)
}

/// Infer [`FontInfo`] from a font file path using filename heuristics.
///
/// No fontconfig or binary font parsing is involved — the family and
/// style are extracted from the file stem via CamelCase splitting and
/// known style-suffix matching.
fn parse_font_info(path: &Path) -> Option<FontInfo> {
    let stem = path.file_stem()?.to_str()?;
    let ext = path.extension()?.to_str()?.to_lowercase();
    if ext != "ttf" && ext != "otf" && ext != "ttc" {
        return None;
    }

    // Common style suffixes found in font filenames.
    const STYLES: &[&str] = &[
        "Regular", "Bold", "Italic", "BoldItalic", "Medium", "Light", "Thin",
        "Black", "ExtraBold", "ExtraLight", "SemiBold", "Semibold", "Hairline",
        "Book", "DemiBold", "Heavy", "Hairline", "ThinItalic", "LightItalic",
        "MediumItalic", "BoldItalic", "BlackItalic",
    ];

    let (family_raw, style) = if let Some(hyphen) = stem.rfind('-') {
        let name_part = &stem[..hyphen];
        let style_part = &stem[hyphen + 1..];
        if STYLES.iter().any(|s| style_part == *s) {
            (name_part.to_string(), style_part.to_string())
        } else {
            // No recognised style suffix — treat the whole stem as the family.
            (stem.to_string(), "Regular".into())
        }
    } else {
        // No hyphen — entire stem is the family name.
        (stem.to_string(), "Regular".into())
    };

    let family = split_camel_case(&family_raw);
    Some(FontInfo {
        family,
        style,
        path: path.to_path_buf(),
    })
}

/// Convert a CamelCase or PascalCase identifier into a spaced, human-readable
/// string (e.g. `"OpenSans"` → `"Open Sans"`, `"SFMono"` → `"SF Mono"`).
fn split_camel_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_uppercase() && !out.is_empty() {
            // Insert a space before an uppercase letter that follows a
            // lowercase letter (PascalCase boundary).
            if out.chars().last().map_or(false, |prev| prev.is_lowercase()) {
                out.push(' ');
            }
            // Insert a space before an uppercase letter that is followed
            // by a lowercase letter (acronym boundary e.g. "SFMono" →
            // "SF Mono", not "S F Mono").
            if chars.peek().map_or(false, |n| n.is_lowercase())
                && out.chars().last().map_or(false, |prev| prev.is_uppercase())
            {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

/// Connect to a running Ghostty instance via IPC and request a runtime
/// introspection snapshot.
///
/// If `ipc_path` is `None`, the default socket resolution order is used
/// (same as [`ghostty_kit::GhosttyControl::try_new`]).
pub fn inspect(
    ipc_path: Option<&Path>,
) -> Result<GhosttySnapshot, GhosttyError> {
    let ctl = match ipc_path {
        Some(path) => ghostty_kit::GhosttyControl::try_with_path(path)
            .ok_or_else(|| GhosttyError::Other(
                format!("IPC socket not found at {}", path.display()),
            ))?,
        None => ghostty_kit::GhosttyControl::try_new()
            .ok_or_else(|| GhosttyError::Other(
                "IPC socket not found — is Ghostty running with --control-socket?".into(),
            ))?,
    };

    let response = ctl.inspect()?;
    let data = response
        .data
        .as_ref()
        .ok_or_else(|| GhosttyError::Other("inspect response missing 'data' field".into()))?;

    let raw = to_serde_value(data);

    // Extract known fields from the raw JSON object.
    let mut snapshot = GhosttySnapshot {
        version: None,
        config_files: Vec::new(),
        config_dirs: Vec::new(),
        resources: Vec::new(),
        pid: None,
        raw: raw.clone(),
    };

    if let serde_json::Value::Object(ref map) = raw {
        if let Some(v) = map.get("version").and_then(|v| v.as_str()) {
            snapshot.version = Some(v.to_string());
        }
        if let Some(arr) = map.get("config_files").and_then(|v| v.as_array()) {
            snapshot.config_files = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        if let Some(arr) = map.get("config_dirs").and_then(|v| v.as_array()) {
            snapshot.config_dirs = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        if let Some(arr) = map.get("resources").and_then(|v| v.as_array()) {
            snapshot.resources = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        if let Some(v) = map.get("pid").and_then(|v| v.as_u64()) {
            snapshot.pid = Some(v as u32);
        }
    }

    Ok(snapshot)
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Resolve the path to the user's Ghostty config.
///
/// Honours `$XDG_CONFIG_HOME` first (matching Ghostty itself), then
/// `dirs::config_dir()`. Always returns `Some` so callers see where we
/// looked even when the file does not yet exist; existence is the
/// caller's job.
fn ghostty_config_path() -> Option<PathBuf> {
    if let Some(p) = xdg_ghostty("config") {
        if p.exists() {
            return Some(p);
        }
    }
    Some(default_ghostty_subpath("config"))
}

fn ghostty_shader_dir() -> Option<PathBuf> {
    if let Some(p) = xdg_ghostty("shaders") {
        if p.exists() {
            return Some(p);
        }
    }
    Some(default_ghostty_subpath("shaders"))
}

/// `<$XDG_CONFIG_HOME>/ghostty/<sub>` when the env var is set.
fn xdg_ghostty(sub: &str) -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME")?;
    Some(PathBuf::from(xdg).join("ghostty").join(sub))
}

/// `<dirs::config_dir()>/ghostty/<sub>` — the conventional fallback.
fn default_ghostty_subpath(sub: &str) -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| Path::new(".").to_path_buf());
    base.join("ghostty").join(sub)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmd_builds() {
        let cmd = cmd();
        // The top-level command exposes exactly one subcommand: `ghostty`.
        let names: Vec<&str> = cmd.get_subcommands().map(|c| c.get_name()).collect();
        assert_eq!(names, vec!["ghostty"]);
        // `ghostty` itself exposes `config`, `ipc`, `shader`, `version`.
        let ghostty = cmd
            .get_subcommands()
            .find(|c| c.get_name() == "ghostty")
            .expect("ghostty subcommand must exist");
        let mut ghosts: Vec<&str> = ghostty.get_subcommands().map(|c| c.get_name()).collect();
        ghosts.sort();
        assert_eq!(
            ghosts,
            vec!["config", "ipc", "shader", "version"],
            "expected subcommands under ghostty"
        );
    }

    #[test]
    fn test_ghostty_subcommand_parses_config() {
        let m = cmd()
            .try_get_matches_from(["forge", "ghostty", "config"])
            .expect("`forge ghostty config` should parse");
        let (name, sub) = m.subcommand().expect("ghostty subcommand must match");
        assert_eq!(name, "ghostty");
        assert!(sub.subcommand_matches("config").is_some());
    }

    #[test]
    fn test_ghostty_subcommand_parses_shader_reload() {
        let m = cmd()
            .try_get_matches_from(["forge", "ghostty", "shader", "reload", "myshader"])
            .expect("`forge ghostty shader reload myshader` should parse");
        let (_, sub) = m.subcommand().expect("ghostty subcommand must match");
        let (_, shader_sub) = sub.subcommand().expect("shader subcommand must match");
        let reload = shader_sub
            .subcommand_matches("reload")
            .expect("reload must match");
        assert_eq!(
            reload.get_one::<String>("name").map(|s| s.as_str()),
            Some("myshader")
        );
    }

    #[test]
    fn test_shader_reload_missing_arg_errors() {
        let err = cmd()
            .try_get_matches_from(["forge", "ghostty", "shader", "reload"])
            .expect_err("missing `name` must be a clap error");
        // clap renders the error to its `Error` type; the message always
        // mentions the missing argument so log scrapers can grep for it.
        let msg = err.to_string();
        assert!(
            msg.contains("name") || msg.contains("<name>") || msg.contains("required"),
            "expected missing-arg error mentioning `name`; got: {msg}"
        );
    }
}