//! `forge ghostty` — user-facing wrapper around the `ghostty-kit` crate.
//!
//! Provides four subcommands:
//!
//! * `status`   — one-line summary: binary on `$PATH`, config file
//!                discoverable, IPC control socket reachable.
//! * `config`   — print the parsed config (auto-discover or an explicit
//!                path) as `key = value` lines, one per line, blank line
//!                between sections.
//! * `reload`   — ask the running Ghostty to reload its config via the
//!                control socket. If no socket is reachable, warn to
//!                stderr and exit 0 (config will apply on next launch).
//! * `validate` — parse a config and surface any warnings, but do not
//!                apply. Exits 1 on parse failure, 0 otherwise.
//!
//! The handler is intentionally non-interactive: it never touches the
//! agent state, conversation state, or the spinner. The only side
//! effects are stdout, stderr, and (for `reload`) one IPC write.

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context as _, Result};
use ghostty_kit::{
    parse_file, ConfigEntry, ConfigValue, GhosttyConfig, GhosttyControl,
};

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

/// Run `forge ghostty status` — print the three-line summary.
///
/// Returns the desired process exit code: `0` if all three rows are
/// present, `1` otherwise. Never panics — IPC probes are wrapped in
/// `GhosttyControl::try_new()` which returns `None` instead of erroring.
pub fn run_status() -> Result<u8> {
    let (binary, binary_path) = detect_binary();
    let (config, config_path) = discover_config();
    let (ipc, ipc_path) = detect_ipc();

    println!("ghostty binary: {}", render_row(binary, &binary_path));
    println!("config file:    {}", render_row(config, &config_path));
    println!("ipc socket:     {}", render_row(ipc, &ipc_path));

    let all_ok = binary && config && ipc;
    Ok(if all_ok { 0 } else { 1 })
}

/// Locate the `ghostty` binary on `$PATH`. Returns `(present, path_or_msg)`.
///
/// Public so integration tests can exercise `detect_binary_in` with
/// controlled PATH values rather than mutating the real environment.
fn detect_binary() -> (bool, String) {
    detect_binary_in(std::env::var_os("PATH").as_deref())
}

/// Walk `path` (as `:`-separated list) looking for an executable file
/// named `ghostty`. Mirrors the resolution `which(1)` uses on POSIX:
/// directory entries that exist but are not executable are silently
/// skipped. Windows is intentionally unsupported — Ghostty itself
/// ships only macOS/Linux builds.
pub fn detect_binary_in(path: Option<&OsStr>) -> (bool, String) {
    let Some(path) = path else {
        return (false, "no".to_string());
    };
    for dir in std::env::split_paths(path) {
        let candidate = dir.join("ghostty");
        if is_executable_file(&candidate) {
            return (true, candidate.to_string_lossy().into_owned());
        }
    }
    (false, "no".to_string())
}

#[cfg(unix)]
fn is_executable_file(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    match std::fs::metadata(p) {
        Ok(md) if md.is_file() => md.permissions().mode() & 0o111 != 0,
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(p: &Path) -> bool {
    p.is_file()
}

/// Discover the active config file. Returns `(present, path_or_msg)`.
fn discover_config() -> (bool, String) {
    match discover_config_path() {
        Some(p) => (true, p.to_string_lossy().into_owned()),
        None => (false, "no".to_string()),
    }
}

/// Resolve a Ghostty config path. Mirrors Ghostty's own resolution
/// order so the CLI matches what the terminal itself would load.
///
/// 1. `$GHOSTTY_CONFIG` if it points to an existing file.
/// 2. `$XDG_CONFIG_HOME/ghostty/config` (default `~/.config/ghostty/config`).
/// 3. `$HOME/Library/Application Support/com.mitchellh.ghostty/config` (macOS).
pub fn discover_config_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("GHOSTTY_CONFIG") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        let p = PathBuf::from(xdg).join("ghostty/config");
        if p.is_file() {
            return Some(p);
        }
    } else if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home).join(".config/ghostty/config");
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home)
            .join("Library/Application Support/com.mitchellh.ghostty/config");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Probe the IPC control socket. Returns `(reachable, hint_or_msg)`.
///
/// We can't read the resolved socket path back from `GhosttyControl`
/// (its `socket_path` field is `pub(crate)`), so the second tuple
/// slot is a *hint* — the platform-default location — regardless of
/// whether the probe succeeded. When the socket is unreachable and
/// no env-var hint applies, the slot collapses to `"no"`.
fn detect_ipc() -> (bool, String) {
    let reachable = GhosttyControl::try_new().is_some();
    let hint = default_socket_hint().unwrap_or_else(|| "no".to_string());
    (reachable, hint)
}

/// Build a one-line `yes (path)` / `no` rendering.
fn render_row(present: bool, path_or_msg: &str) -> String {
    if present {
        format!("yes ({path_or_msg})")
    } else {
        path_or_msg.to_string()
    }
}

/// Best-effort platform-default socket location for status hints.
pub fn default_socket_hint() -> Option<String> {
    if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Some(
            PathBuf::from(xdg)
                .join("ghostty/control.sock")
                .to_string_lossy()
                .into_owned(),
        );
    }
    if let Some(tmp) = std::env::var_os("TMPDIR") {
        return Some(
            PathBuf::from(tmp)
                .join("ghostty-control.sock")
                .to_string_lossy()
                .into_owned(),
        );
    }
    Some("/tmp/ghostty-control.sock".to_string())
}

// ---------------------------------------------------------------------------
// config show
// ---------------------------------------------------------------------------

/// Run `forge ghostty show [path]` — print parsed config.
///
/// If `path` is `None`, the active config is discovered via
/// `GhosttyConfig::discover()` (mirrors `discover_config_path` here).
/// Returns `Ok(1)` on parse failure or missing config — the caller
/// maps that to process exit.
pub fn run_show(path: Option<&Path>) -> Result<u8> {
    let resolved = match path {
        Some(p) => p.to_path_buf(),
        None => discover_config_path().ok_or_else(|| {
            anyhow!(
                "no Ghostty config found (set $GHOSTTY_CONFIG or create \
                 ~/.config/ghostty/config)"
            )
        })?,
    };

    let config = match parse_file(&resolved) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to parse {}: {e}", resolved.display());
            return Ok(1);
        }
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    print_config(&config, &mut out);
    let _ = out.flush();
    Ok(0)
}

/// Print a parsed config as `key = value` lines, blank line between
/// sections. Takes an arbitrary `Write` so tests can capture output
/// without touching real stdout.
pub fn print_config<W: Write>(config: &GhosttyConfig, out: &mut W) {
    let mut current_section: Option<String> = None;

    for entry in &config.entries {
        match entry {
            ConfigEntry::KeyValue {
                key,
                value,
                section,
                ..
            } => {
                if *section != current_section {
                    if current_section.is_some() {
                        let _ = writeln!(out);
                    }
                    current_section = section.clone();
                    if let Some(s) = section {
                        let _ = writeln!(out, "[{s}]");
                    }
                }
                let _ = writeln!(out, "{} = {}", key, render_value(value));
            }
            ConfigEntry::Section(name, _) => {
                if current_section.is_some() {
                    let _ = writeln!(out);
                }
                current_section = Some(name.clone());
                let _ = writeln!(out, "[{name}]");
            }
            ConfigEntry::Include(p) => {
                let _ = writeln!(out, "config-file = {}", p.display());
            }
        }
    }
}

fn render_value(value: &ConfigValue) -> String {
    match value {
        ConfigValue::String(s) => s.clone(),
        ConfigValue::Bool(b) => b.to_string(),
        ConfigValue::Integer(n) => n.to_string(),
        ConfigValue::Color(rgba) => format!("#{rgba:08X}"),
        ConfigValue::List(items) => items.join(", "),
    }
}

// ---------------------------------------------------------------------------
// reload
// ---------------------------------------------------------------------------

/// Run `forge ghostty reload` — ask the running Ghostty to reload.
///
/// Probes the IPC control socket via `GhosttyControl::try_new()`. If
/// the socket is unreachable, prints a warning to stderr and exits 0
/// (config will apply on next launch). Returns `Ok(0)` in both cases
/// — reload is best-effort.
pub fn run_reload() -> Result<u8> {
    let Some(ctl) = GhosttyControl::try_new() else {
        eprintln!(
            "warning: Ghostty control socket not reachable; \
             config will apply on next launch"
        );
        return Ok(0);
    };

    ctl.reload_config().context("reload_config IPC call failed")?;

    let path = discover_config_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "(unknown)".to_string());
    println!("reloaded config at {path}");
    Ok(0)
}

// ---------------------------------------------------------------------------
// validate
// ---------------------------------------------------------------------------

/// Run `forge ghostty validate <path>` — parse and surface warnings.
///
/// Returns `Ok(0)` for a valid file (with or without warnings) and
/// `Ok(1)` on parse failure. Both cases print to stderr — warnings
/// one-per-line, errors as a single `error: …` line.
pub fn run_validate(path: &Path) -> Result<u8> {
    let config = match parse_file(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(1);
        }
    };

    for w in validate_config(&config, path) {
        eprintln!("{w}");
    }
    Ok(0)
}

/// Parse-free validation pass: returns warnings that should be printed
/// to stderr for a successfully-parsed config. Returns `Err` if the
/// file cannot be parsed (mirrors `run_validate`'s exit-1 path).
pub fn validate_warnings(path: &Path) -> Result<Vec<String>> {
    let config = parse_file(path).with_context(|| {
        format!("failed to parse {}", path.display())
    })?;
    Ok(validate_config(&config, path))
}

fn validate_config(config: &GhosttyConfig, path: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    for entry in &config.entries {
        if let ConfigEntry::KeyValue {
            key,
            value: ConfigValue::String(s),
            line,
            ..
        } = entry
        {
            if looks_like_failed_color(s) {
                warnings.push(format!(
                    "warning: {}:{line}: value `{s}` for `{key}` looks like \
                     a color literal but is not #RRGGBB[AA]",
                    path.display()
                ));
            }
        }
    }
    warnings
}

fn looks_like_failed_color(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('#') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    if rest.chars().all(|c| c.is_ascii_hexdigit()) {
        return !matches!(rest.len(), 6 | 8);
    }
    rest.chars().any(|c| c.is_ascii_hexdigit())
}
