//! # forge_pheno_shell
//!
//! Shell abstraction layer for forgecode (per ADR-101 §4.1, ADR-096 fleet pattern).
//!
//! Detects the user's shell, emits shell-specific completion scripts, and routes
//! environment setup per shell. Supports:
//!
//! - **POSIX**: ZSH, Bash, Fish, Tcsh, Oil, Elvish, Nushell
//! - **Windows-native**: PowerShell (Windows), PowerShell Core (cross-platform), Cmd
//! - **Emulator shells**: WSL Bash (Windows -> Linux), Git Bash (Windows)
//!
//! This crate is intentionally **zero dependency on `forge_domain`** (ADR-097 decoupling
//! pattern). It is pure-Rust, framework-agnostic, and consumable from any forgecode crate.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// All shells forgecode knows how to detect and emit completions for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShellKind {
    /// ZSH — primary shell on macOS + most developer Linux boxes.
    Zsh,
    /// Bash — universal on Linux + Git Bash on Windows + WSL.
    Bash,
    /// Fish — popular on Linux + macOS developer machines.
    Fish,
    /// PowerShell on Windows (powershell.exe, Windows PowerShell 5.1).
    PowerShellWindows,
    /// PowerShell Core (pwsh, cross-platform: macOS/Linux/Windows).
    PowerShellCore,
    /// Cmd.exe — Windows default command interpreter.
    Cmd,
    /// Nushell (`nu`) — modern data-oriented shell, cross-platform.
    Nushell,
    /// Elvish — Go-based shell with structured pipelines.
    Elvish,
    /// Tcsh / Csh — BSD-derived C shell.
    Tcsh,
    /// Oil / Oils — POSIX-compatible bash alternative.
    Oil,
    /// WSL bash — bash running inside Windows Subsystem for Linux.
    WslBash,
    /// Git Bash — bash bundled with Git for Windows.
    GitBash,
    /// Unknown / not detected. We always have a fallback.
    Unknown,
}

impl ShellKind {
    /// Stable identifier used in config files and telemetry.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
            Self::PowerShellWindows => "powershell-windows",
            Self::PowerShellCore => "powershell-core",
            Self::Cmd => "cmd",
            Self::Nushell => "nushell",
            Self::Elvish => "elvish",
            Self::Tcsh => "tcsh",
            Self::Oil => "oil",
            Self::WslBash => "wsl-bash",
            Self::GitBash => "git-bash",
            Self::Unknown => "unknown",
        }
    }

    /// POSIX-class shells (treat as POSIX for env, paths, completion).
    pub fn is_posix(&self) -> bool {
        matches!(
            self,
            Self::Zsh
                | Self::Bash
                | Self::Fish
                | Self::Nushell
                | Self::Elvish
                | Self::Oil
                | Self::WslBash
                | Self::GitBash
        )
    }

    /// Windows-native shells.
    pub fn is_windows_native(&self) -> bool {
        matches!(self, Self::PowerShellWindows | Self::Cmd)
    }

    /// Supports shell-completion script generation.
    pub fn supports_completions(&self) -> bool {
        // All known shells except Cmd and Unknown.
        !matches!(self, Self::Cmd | Self::Unknown)
    }

    /// Family grouping for the env-var resolution table.
    pub fn family(&self) -> ShellFamily {
        match self {
            Self::Zsh | Self::Bash | Self::WslBash | Self::GitBash | Self::Oil => {
                ShellFamily::Sh
            }
            Self::Fish => ShellFamily::Fish,
            Self::PowerShellWindows | Self::PowerShellCore => ShellFamily::PowerShell,
            Self::Cmd => ShellFamily::Cmd,
            Self::Nushell => ShellFamily::Nushell,
            Self::Elvish => ShellFamily::Elvish,
            Self::Tcsh => ShellFamily::Tcsh,
            Self::Unknown => ShellFamily::Unknown,
        }
    }

    /// All known shells (for tests, registry builders, completion installers).
    ///
    /// Includes the catch-all [`ShellKind::Unknown`] sentinel as the last
    /// element so callers can rely on `all().len()` being the total
    /// number of variants in the enum.
    pub fn all() -> &'static [ShellKind] {
        &[
            Self::Zsh,
            Self::Bash,
            Self::Fish,
            Self::PowerShellWindows,
            Self::PowerShellCore,
            Self::Cmd,
            Self::Nushell,
            Self::Elvish,
            Self::Tcsh,
            Self::Oil,
            Self::WslBash,
            Self::GitBash,
            Self::Unknown,
        ]
    }
}

impl fmt::Display for ShellKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// Shell family grouping (coarser than `ShellKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShellFamily {
    /// sh-derived: ZSH, Bash, WSL Bash, Git Bash, Oil.
    Sh,
    /// Fish.
    Fish,
    /// PowerShell (Windows + Core).
    PowerShell,
    /// Windows Cmd.
    Cmd,
    /// Nushell.
    Nushell,
    /// Elvish.
    Elvish,
    /// Tcsh / Csh.
    Tcsh,
    /// Unknown.
    Unknown,
}

/// Where the shell was detected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellDetection {
    /// What kind of shell.
    pub kind: ShellKind,
    /// Source of detection (for debugging + telemetry).
    pub source: DetectionSource,
    /// Raw value that triggered detection (e.g. `$SHELL`, `$PSVersionTable.PSEdition`).
    pub raw: String,
}

/// Where the shell detection came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DetectionSource {
    /// `$SHELL` env var on POSIX.
    PosixShellEnv,
    /// `$0` on POSIX (login shell name).
    PosixArgv0,
    /// PowerShell's `$PSVersionTable.PSEdition`.
    PowerShellEdition,
    /// `cmd.exe /C echo %COMSPEC%`.
    WindowsComspec,
    /// WSL-specific: `/proc/version` contains "microsoft" or "WSL".
    WslProcVersion,
    /// Caller explicitly named the shell (config override, test fixture).
    Explicit,
    /// Best-effort fallback when nothing else matched.
    Fallback,
}

/// Errors produced by forge_pheno_shell. None of these are I/O errors
/// during install — install success is reported via [`InstallResult::Written`].
/// These only signal that the requested operation is structurally invalid
/// for the detected shell.
#[derive(Debug, Error)]
pub enum ShellError {
    /// Detection failed entirely. `PHENO_SHELL_KIND` is unset, `argv[0]`
    /// doesn't end with a recognized shell name, and `$PSEdition` /
    /// `COMSPEC` don't indicate Windows shell.
    #[error("could not detect shell from environment (tried PHENO_SHELL_KIND, argv[0], $PSEdition, COMSPEC)")]
    DetectionFailed,
    /// Requested a completion for a shell that doesn't support completion emission.
    #[error("shell {kind} does not support completion emission")]
    CompletionUnsupported { kind: ShellKind },
}

/// Detected shell environment.
#[derive(Debug, Clone)]
pub struct ShellEnv {
    /// Detected kind.
    pub kind: ShellKind,
    /// Detected family.
    pub family: ShellFamily,
    /// Full detection record (for telemetry + `--debug-shell`).
    pub detection: ShellDetection,
    /// Resolved env vars per shell family (PATH, HOME, EDITOR, etc.).
    pub vars: ShellVars,
}

/// Shell-family-specific env vars.
#[derive(Debug, Clone, Default)]
pub struct ShellVars {
    /// Path list separator (`:` on POSIX, `;` on Windows).
    pub path_separator: String,
    /// Env var holding the executable search path.
    pub path_var: String,
    /// Env var holding the user's home directory.
    pub home_var: String,
    /// Env var holding the editor.
    pub editor_var: String,
    /// Line continuation char (`\` on POSIX, `` ` `` on Cmd, `` ` `` on PowerShell).
    pub line_continuation: String,
}

impl ShellVars {
    /// Resolve the env var name set for a given shell family.
    pub fn for_family(family: ShellFamily) -> Self {
        match family {
            ShellFamily::Sh | ShellFamily::Fish | ShellFamily::Nushell | ShellFamily::Elvish => {
                Self {
                    path_separator: ":".into(),
                    path_var: "PATH".into(),
                    home_var: "HOME".into(),
                    editor_var: "EDITOR".into(),
                    line_continuation: "\\".into(),
                }
            }
            ShellFamily::PowerShell => Self {
                path_separator: ";".into(),
                path_var: "PATH".into(),
                home_var: "USERPROFILE".into(),
                editor_var: "EDITOR".into(),
                line_continuation: "`".into(),
            },
            ShellFamily::Cmd => Self {
                path_separator: ";".into(),
                path_var: "PATH".into(),
                home_var: "USERPROFILE".into(),
                editor_var: "EDITOR".into(),
                line_continuation: "^".into(),
            },
            ShellFamily::Tcsh => Self {
                path_separator: ":".into(),
                path_var: "PATH".into(),
                home_var: "HOME".into(),
                editor_var: "EDITOR".into(),
                line_continuation: "\\".into(),
            },
            ShellFamily::Unknown => Self::default(),
        }
    }
}

/// Detect the shell from environment + argv. Pure function — no IO beyond
/// reading env vars and (optionally) `/proc/version` on Linux.
pub fn detect_shell(env: &std::collections::HashMap<String, String>, argv0: Option<&str>) -> Result<ShellEnv, ShellError> {
    // Priority 1: explicit override (for tests + config).
    if let Some(explicit) = env.get("FORGE_SHELL") {
        return Ok(from_explicit(explicit));
    }
    if let Some(arg0) = argv0 {
        if let Some(kind) = detect_from_argv0(arg0) {
            return Ok(ShellEnv {
                kind,
                family: kind.family(),
                detection: ShellDetection {
                    kind,
                    source: DetectionSource::PosixArgv0,
                    raw: arg0.to_string(),
                },
                vars: ShellVars::for_family(kind.family()),
            });
        }
    }
    // Priority 2: PowerShell edition (Windows + Core).
    if let Some(edition) = env.get("PSEdition") {
        let kind = match edition.as_str() {
            "Desktop" => ShellKind::PowerShellWindows,
            "Core" => ShellKind::PowerShellCore,
            _ => return Err(ShellError::DetectionFailed),
        };
        return Ok(ShellEnv {
            kind,
            family: kind.family(),
            detection: ShellDetection {
                kind,
                source: DetectionSource::PowerShellEdition,
                raw: edition.clone(),
            },
            vars: ShellVars::for_family(kind.family()),
        });
    }
    // Priority 3: COMSPEC on Windows (Cmd).
    if let Some(comspec) = env.get("COMSPEC") {
        if comspec.to_lowercase().contains("cmd") {
            let kind = ShellKind::Cmd;
            return Ok(ShellEnv {
                kind,
                family: kind.family(),
                detection: ShellDetection {
                    kind,
                    source: DetectionSource::WindowsComspec,
                    raw: comspec.clone(),
                },
                vars: ShellVars::for_family(kind.family()),
            });
        }
    }
    // Priority 4: SHELL on POSIX.
    if let Some(shell) = env.get("SHELL") {
        return Ok(ShellEnv {
            kind: detect_from_path(shell).unwrap_or(ShellKind::Unknown),
            family: ShellFamily::Sh,
            detection: ShellDetection {
                kind: detect_from_path(shell).unwrap_or(ShellKind::Unknown),
                source: DetectionSource::PosixShellEnv,
                raw: shell.clone(),
            },
            vars: ShellVars::for_family(ShellFamily::Sh),
        });
    }
    Err(ShellError::DetectionFailed)
}

fn detect_from_argv0(arg0: &str) -> Option<ShellKind> {
    // Try POSIX path separator first, then Windows backslash (for cross-platform parsing)
    let base = if let Some((_, tail)) = arg0.rsplit_once('\\') {
        tail
    } else {
        std::path::Path::new(arg0)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(arg0)
    };
    match base {
        "zsh" => Some(ShellKind::Zsh),
        "bash" => Some(ShellKind::Bash),
        "fish" => Some(ShellKind::Fish),
        "pwsh" => Some(ShellKind::PowerShellCore),
        "powershell" | "powershell.exe" => Some(ShellKind::PowerShellWindows),
        "cmd" | "cmd.exe" => Some(ShellKind::Cmd),
        "nu" => Some(ShellKind::Nushell),
        "elvish" => Some(ShellKind::Elvish),
        "tcsh" | "csh" => Some(ShellKind::Tcsh),
        "osh" | "oil" => Some(ShellKind::Oil),
        _ => None,
    }
}

fn detect_from_path(shell_path: &str) -> Option<ShellKind> {
    detect_from_argv0(shell_path)
}

fn from_explicit(explicit: &str) -> ShellEnv {
    let kind = match explicit {
        "zsh" => ShellKind::Zsh,
        "bash" => ShellKind::Bash,
        "fish" => ShellKind::Fish,
        "powershell-windows" | "powershell" => ShellKind::PowerShellWindows,
        "powershell-core" | "pwsh" => ShellKind::PowerShellCore,
        "cmd" => ShellKind::Cmd,
        "nushell" | "nu" => ShellKind::Nushell,
        "elvish" => ShellKind::Elvish,
        "tcsh" => ShellKind::Tcsh,
        "oil" => ShellKind::Oil,
        "wsl-bash" => ShellKind::WslBash,
        "git-bash" => ShellKind::GitBash,
        _ => ShellKind::Unknown,
    };
    let family = kind.family();
    ShellEnv {
        kind,
        family,
        detection: ShellDetection {
            kind,
            source: DetectionSource::Explicit,
            raw: explicit.to_string(),
        },
        vars: ShellVars::for_family(family),
    }
}

/// Generate a shell-specific completion script.
///
/// Returns a string containing the script source, ready to be written to
/// `~/.zsh/completions/_forge`, `~/.bash_completion.d/forge`, etc.
pub fn completion_script(kind: ShellKind, binary_name: &str) -> Result<String, ShellError> {
    if !kind.supports_completions() {
        return Err(ShellError::CompletionUnsupported { kind });
    }
    Ok(match kind {
        ShellKind::Zsh => zsh_completion(binary_name),
        ShellKind::Bash | ShellKind::WslBash | ShellKind::GitBash => bash_completion(binary_name),
        ShellKind::Fish => fish_completion(binary_name),
        ShellKind::PowerShellWindows | ShellKind::PowerShellCore => {
            powershell_completion(binary_name)
        }
        ShellKind::Nushell => nushell_completion(binary_name),
        ShellKind::Elvish => elvish_completion(binary_name),
        ShellKind::Oil => bash_completion(binary_name), // Oil is bash-compatible
        ShellKind::Tcsh => tcsh_completion(binary_name),
        // Cmd and Unknown already filtered by `supports_completions`.
        ShellKind::Cmd | ShellKind::Unknown => unreachable!(),
    })
}

fn zsh_completion(bin: &str) -> String {
    format!(
        r#"#compdef {bin}
# ZSH completion for {bin} (generated by forge_pheno_shell v0.1.0)

_{bin}() {{
    local -a subcommands
    subcommands=(
        'chat:Start an interactive chat session'
        'run:Run a single prompt non-interactively'
        'init:Initialize forgecode in the current shell'
        'config:View or edit configuration'
        'provider:Manage LLM providers'
        'session:Manage sessions'
        'memory:Query or clear memory'
        'plugin:Install or remove plugins (pheno-forge-plugins compatible)'
        'completion:Generate shell completion scripts'
        'doctor:Diagnose installation + sidecar health'
        'version:Print version'
    )

    _arguments -s \
        '1: :->cmd' \
        '*::arg:->args'

    case "$state" in
        cmd)
            _describe -t commands 'forge subcommand' subcommands
            ;;
        args)
            case $words[1] in
                provider)
                    _arguments '1: :(add list remove test)'
                    ;;
                memory)
                    _arguments '1: :(store recall forget list scopes)' \
                              '--scope[Memory scope]:scope:(episodic identity project_knowledge fallback)'
                    ;;
                plugin)
                    _arguments '1: :(install list enable disable info)' \
                              '--from-tarball[Install from local tarball]:file:_files'
                    ;;
            esac
            ;;
    esac
}}

compdef _{bin} {bin}
"#
    )
}

fn bash_completion(bin: &str) -> String {
    format!(
        r#"# Bash completion for {bin} (generated by forge_pheno_shell v0.1.0)

_{bin}() {{
    local cur prev cmds
    COMPREPLY=()
    cur="${{COMP_WORDS[COMP_CWORD]}}"
    prev="${{COMP_WORDS[COMP_CWORD-1]}}"
    cmds="chat run init config provider session memory plugin completion doctor version"

    if [[ $COMP_CWORD -eq 1 ]]; then
        COMPREPLY=( $(compgen -W "$cmds" -- "$cur") )
        return 0
    fi

    case "${{COMP_WORDS[1]}}" in
        provider)
            COMPREPLY=( $(compgen -W "add list remove test" -- "$cur") )
            ;;
        memory)
            if [[ "$prev" == "--scope" ]]; then
                COMPREPLY=( $(compgen -W "episodic identity project_knowledge fallback" -- "$cur") )
            else
                COMPREPLY=( $(compgen -W "store recall forget list scopes --scope" -- "$cur") )
            fi
            ;;
        plugin)
            COMPREPLY=( $(compgen -W "install list enable disable info --from-tarball" -- "$cur") )
            ;;
    esac
    return 0
}}

complete -F _{bin} {bin}
"#
    )
}

fn fish_completion(bin: &str) -> String {
    format!(
        r#"# Fish completion for {bin} (generated by forge_pheno_shell v0.1.0)

function _{bin}_subcommands
    echo -e "chat\nrun\ninit\nconfig\nprovider\nsession\nmemory\nplugin\ncompletion\ndoctor\nversion"
end

function _{bin}
    set -l cmd (commandline -opc)
    set -l cur (commandline -ct)

    if test (count $cmd) -eq 1
        complete -c {bin} -f -a "({bin}_subcommands)"
    else
        switch $cmd[2]
            case provider
                complete -c {bin} -f -a "add list remove test"
            case memory
                complete -c {bin} -f -l scope -a "episodic identity project_knowledge fallback"
                complete -c {bin} -f -a "store recall forget list scopes"
            case plugin
                complete -c {bin} -f -l from-tarball -r
                complete -c {bin} -f -a "install list enable disable info"
        end
    end
end

complete -c {bin} -f -a "({bin}_subcommands)" -d "forgecode subcommand"
"#
    )
}

fn powershell_completion(bin: &str) -> String {
    format!(
        r#"# PowerShell completion for {bin} (generated by forge_pheno_shell v0.1.0)
# Works in PowerShell Windows + PowerShell Core (pwsh).

using namespace System.Management.Automation

Register-ArgumentCompleter -Native -CommandName '{bin}' -ScriptBlock {{
    param($wordToComplete, $commandAst, $cursorPosition)

    $subcommands = @(
        @{{ Name = 'chat';       Description = 'Start an interactive chat session' }}
        @{{ Name = 'run';        Description = 'Run a single prompt non-interactively' }}
        @{{ Name = 'init';       Description = 'Initialize forgecode in the current shell' }}
        @{{ Name = 'config';     Description = 'View or edit configuration' }}
        @{{ Name = 'provider';   Description = 'Manage LLM providers' }}
        @{{ Name = 'session';    Description = 'Manage sessions' }}
        @{{ Name = 'memory';     Description = 'Query or clear memory' }}
        @{{ Name = 'plugin';     Description = 'Install or remove plugins' }}
        @{{ Name = 'completion'; Description = 'Generate shell completion scripts' }}
        @{{ Name = 'doctor';     Description = 'Diagnose installation + sidecar health' }}
        @{{ Name = 'version';    Description = 'Print version' }}
    )

    if ($commandAst.CommandElements.Count -eq 1) {{
        $subcommands | Where-Object {{ $_.Name -like "$wordToComplete*" }} | ForEach-Object {{
            [System.Management.Automation.CompletionResult]::new(
                $_.Name, $_.Name, 'ParameterName', $_.Description
            )
        }}
        return
    }}

    switch ($commandAst.CommandElements[1].Extent.Text) {{
        'provider' {{
            @('add','list','remove','test') | Where-Object {{ $_ -like "$wordToComplete*" }} | ForEach-Object {{
                [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
            }}
        }}
        'memory' {{
            @('store','recall','forget','list','scopes') | Where-Object {{ $_ -like "$wordToComplete*" }} | ForEach-Object {{
                [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
            }}
        }}
        'plugin' {{
            @('install','list','enable','disable','info') | Where-Object {{ $_ -like "$wordToComplete*" }} | ForEach-Object {{
                [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
            }}
        }}
    }}
}}
"#
    )
}

fn nushell_completion(bin: &str) -> String {
    format!(
        r#"# Nushell completion for {bin} (generated by forge_pheno_shell v0.1.0)

export extern "{bin}" [
    --help(-h)                       # Show help
    --version(-V)                    # Show version
    --shell(-s):string               # Override shell detection
    --bridge-path:path               # Path to libpheno_bridge dylib
    --mode: string                   # Mock or sidecar (pheno-forge-smoke)
    --scope: string                  # Memory scope (episodic/identity/project_knowledge/fallback)
    subcommand?: string              # chat|run|init|config|provider|session|memory|plugin|completion|doctor|version
    ...args
]
"#
    )
}

fn elvish_completion(bin: &str) -> String {
    format!(
        r#"use builtin;
use str;

set edit:completion:arg-completer[{bin}] = {{|@args|
    fn spaces {{|n| builtin:repeat $n ' ' }}
    fn cand {{|text desc| edit:complex-candidate $text $desc }}
    var command = '{bin}'
    var subcmds = [
        &'chat='      'Start an interactive chat session'
        &'run='       'Run a single prompt non-interactively'
        &'init='      'Initialize forgecode in the current shell'
        &'config='    'View or edit configuration'
        &'provider='  'Manage LLM providers'
        &'session='   'Manage sessions'
        &'memory='    'Query or clear memory'
        &'plugin='    'Install or remove plugins'
        &'completion=' 'Generate shell completion scripts'
        &'doctor='    'Diagnose installation + sidecar health'
        &'version='   'Print version'
    ]
    var completions = []{{}}
    edit:redraw &full=$false
    $completions
}}
"#
    )
}

fn tcsh_completion(bin: &str) -> String {
    format!(
        r#"# Tcsh completion for {bin} (generated by forge_pheno_shell v0.1.0)

complete {bin} \
    'c/chat/(Start an interactive chat session)/' \
    'c/run/(Run a single prompt non-interactively)/' \
    'c/init/(Initialize forgecode in the current shell)/' \
    'c/config/(View or edit configuration)/' \
    'c/provider/(Manage LLM providers)/' \
    'c/session/(Manage sessions)/' \
    'c/memory/(Query or clear memory)/' \
    'c/plugin/(Install or remove plugins)/' \
    'c/completion/(Generate shell completion scripts)/' \
    'c/doctor/(Diagnose installation + sidecar health)/' \
    'c/version/(Print version)/' \
    'n--scope/(episodic identity project_knowledge fallback)/' \
    'n--mode/(mock sidecar)/'
"#
    )
}

/// Where the completion script should be installed (per shell).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionInstallTarget {
    /// Absolute path to write the script to.
    pub path: String,
    /// Human-readable description (for `--list-install-targets`).
    pub description: String,
}

/// Compute where to install a completion script on the current machine.
///
/// Returns an empty Vec on shells that don't support completions.
pub fn install_targets(kind: ShellKind, home_dir: &std::path::Path, bin: &str) -> Vec<CompletionInstallTarget> {
    if !kind.supports_completions() {
        return Vec::new();
    }
    let path = match kind {
        ShellKind::Zsh => home_dir.join(".zsh/completions").join(format!("_{bin}")),
        ShellKind::Bash | ShellKind::WslBash | ShellKind::GitBash => {
            home_dir.join(".bash_completion.d").join(bin)
        }
        ShellKind::Fish => home_dir.join(".config/fish/completions").join(format!("{bin}.fish")),
        ShellKind::PowerShellWindows => std::path::PathBuf::from(format!(
            "$HOME\\Documents\\PowerShell\\Microsoft.PowerShell_profile.ps1"
        )),
        ShellKind::PowerShellCore => {
            // XDG-friendly: ~/.local/share/powershell/Completions/<bin>.ps1
            home_dir
                .join(".local/share/powershell/Completions")
                .join(format!("{bin}.ps1"))
        }
        ShellKind::Nushell => home_dir
            .join(".config/nushell")
            .join(format!("completions-{bin}.nu")),
        ShellKind::Elvish => home_dir.join(".elvish/lib").join(format!("{bin}.elv")),
        ShellKind::Oil => home_dir.join(".oil/completions").join(bin), // Oil uses bash-compat
        ShellKind::Tcsh => home_dir.join(".tcsh_completions").join(bin),
        ShellKind::Cmd | ShellKind::Unknown => return Vec::new(),
    };
    vec![CompletionInstallTarget {
        path: path.to_string_lossy().to_string(),
        description: format!("Completion for {} ({} style)", bin, kind.id()),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_kind_id_is_stable() {
        // These IDs are persisted in config files; do not change.
        assert_eq!(ShellKind::Zsh.id(), "zsh");
        assert_eq!(ShellKind::PowerShellWindows.id(), "powershell-windows");
        assert_eq!(ShellKind::PowerShellCore.id(), "powershell-core");
        assert_eq!(ShellKind::WslBash.id(), "wsl-bash");
    }

    #[test]
    fn is_posix_classification() {
        assert!(ShellKind::Zsh.is_posix());
        assert!(ShellKind::Bash.is_posix());
        assert!(ShellKind::Fish.is_posix());
        assert!(ShellKind::Nushell.is_posix());
        assert!(!ShellKind::PowerShellWindows.is_posix());
        assert!(!ShellKind::PowerShellCore.is_posix());
        assert!(!ShellKind::Cmd.is_posix());
    }

    #[test]
    fn is_windows_native_classification() {
        assert!(ShellKind::PowerShellWindows.is_windows_native());
        assert!(ShellKind::Cmd.is_windows_native());
        assert!(!ShellKind::PowerShellCore.is_windows_native()); // cross-platform
        assert!(!ShellKind::Bash.is_windows_native());
    }

    #[test]
    fn supports_completions() {
        assert!(ShellKind::Zsh.supports_completions());
        assert!(ShellKind::PowerShellWindows.supports_completions());
        assert!(ShellKind::Nushell.supports_completions());
        assert!(!ShellKind::Cmd.supports_completions());
        assert!(!ShellKind::Unknown.supports_completions());
    }

    #[test]
    fn all_kinds_count() {
        // 12 known shells + Unknown = 13.
        assert_eq!(ShellKind::all().len(), 13);
    }

    #[test]
    fn shell_vars_posix() {
        let vars = ShellVars::for_family(ShellFamily::Sh);
        assert_eq!(vars.path_separator, ":");
        assert_eq!(vars.path_var, "PATH");
        assert_eq!(vars.home_var, "HOME");
    }

    #[test]
    fn shell_vars_powershell() {
        let vars = ShellVars::for_family(ShellFamily::PowerShell);
        assert_eq!(vars.path_separator, ";");
        assert_eq!(vars.home_var, "USERPROFILE");
    }

    #[test]
    fn shell_vars_cmd() {
        let vars = ShellVars::for_family(ShellFamily::Cmd);
        assert_eq!(vars.path_separator, ";");
        assert_eq!(vars.line_continuation, "^");
    }

    #[test]
    fn detect_from_argv0_zsh() {
        let env = std::collections::HashMap::new();
        let result = detect_shell(&env, Some("/bin/zsh")).unwrap();
        assert_eq!(result.kind, ShellKind::Zsh);
        assert_eq!(result.detection.source, DetectionSource::PosixArgv0);
    }

    #[test]
    fn detect_from_argv0_pwsh() {
        let env = std::collections::HashMap::new();
        let result = detect_shell(&env, Some("/usr/local/bin/pwsh")).unwrap();
        assert_eq!(result.kind, ShellKind::PowerShellCore);
    }

    #[test]
    fn detect_from_argv0_cmd() {
        let env = std::collections::HashMap::new();
        let result = detect_shell(&env, Some("C:\\Windows\\System32\\cmd.exe")).unwrap();
        assert_eq!(result.kind, ShellKind::Cmd);
    }

    #[test]
    fn detect_from_argv0_nushell() {
        let env = std::collections::HashMap::new();
        let result = detect_shell(&env, Some("/opt/homebrew/bin/nu")).unwrap();
        assert_eq!(result.kind, ShellKind::Nushell);
    }

    #[test]
    fn detect_from_argv0_elvish() {
        let env = std::collections::HashMap::new();
        let result = detect_shell(&env, Some("/usr/bin/elvish")).unwrap();
        assert_eq!(result.kind, ShellKind::Elvish);
    }

    #[test]
    fn detect_via_shell_env() {
        let mut env = std::collections::HashMap::new();
        env.insert("SHELL".into(), "/bin/fish".into());
        let result = detect_shell(&env, None).unwrap();
        assert_eq!(result.kind, ShellKind::Fish);
        assert_eq!(result.detection.source, DetectionSource::PosixShellEnv);
    }

    #[test]
    fn detect_via_psedition_desktop() {
        let mut env = std::collections::HashMap::new();
        env.insert("PSEdition".into(), "Desktop".into());
        let result = detect_shell(&env, None).unwrap();
        assert_eq!(result.kind, ShellKind::PowerShellWindows);
    }

    #[test]
    fn detect_via_psedition_core() {
        let mut env = std::collections::HashMap::new();
        env.insert("PSEdition".into(), "Core".into());
        let result = detect_shell(&env, None).unwrap();
        assert_eq!(result.kind, ShellKind::PowerShellCore);
    }

    #[test]
    fn detect_via_comspec() {
        let mut env = std::collections::HashMap::new();
        env.insert("COMSPEC".into(), "C:\\Windows\\System32\\cmd.exe".into());
        let result = detect_shell(&env, None).unwrap();
        assert_eq!(result.kind, ShellKind::Cmd);
        assert_eq!(result.detection.source, DetectionSource::WindowsComspec);
    }

    #[test]
    fn explicit_override_takes_priority() {
        let mut env = std::collections::HashMap::new();
        env.insert("SHELL".into(), "/bin/bash".into());
        env.insert("FORGE_SHELL".into(), "zsh".into());
        let result = detect_shell(&env, Some("/bin/bash")).unwrap();
        assert_eq!(result.kind, ShellKind::Zsh);
        assert_eq!(result.detection.source, DetectionSource::Explicit);
    }

    #[test]
    fn explicit_aliases_accepted() {
        let mut env = std::collections::HashMap::new();
        env.insert("FORGE_SHELL".into(), "pwsh".into());
        let result = detect_shell(&env, None).unwrap();
        assert_eq!(result.kind, ShellKind::PowerShellCore);
    }

    #[test]
    fn detection_fails_with_no_signals() {
        let env = std::collections::HashMap::new();
        let result = detect_shell(&env, None);
        assert!(matches!(result, Err(ShellError::DetectionFailed)));
    }

    #[test]
    fn zsh_completion_contains_compdef_and_subcommands() {
        let script = completion_script(ShellKind::Zsh, "forge").unwrap();
        assert!(script.contains("#compdef forge"));
        assert!(script.contains("compdef _forge forge"));
        assert!(script.contains("'memory:Query or clear memory'"));
        assert!(script.contains("--scope"));
        assert!(script.contains("project_knowledge"));
    }

    #[test]
    fn bash_completion_contains_complete_and_subcommands() {
        let script = completion_script(ShellKind::Bash, "forge").unwrap();
        assert!(script.contains("complete -F _forge forge"));
        assert!(script.contains("cmds=\"chat run init config"));
        assert!(script.contains("provider)"));
        assert!(script.contains("memory)"));
    }

    #[test]
    fn fish_completion_contains_function_and_subcommands() {
        let script = completion_script(ShellKind::Fish, "forge").unwrap();
        assert!(script.contains("function _forge"));
        assert!(script.contains("commandline -opc"));
        assert!(script.contains("complete -c forge"));
    }

    #[test]
    fn powershell_completion_uses_register_argument_completer() {
        let script = completion_script(ShellKind::PowerShellWindows, "forge").unwrap();
        assert!(script.contains("Register-ArgumentCompleter"));
        assert!(script.contains("-CommandName 'forge'"));
        assert!(script.contains("Management.Automation"));
        let script2 = completion_script(ShellKind::PowerShellCore, "forge").unwrap();
        assert!(script2.contains("Register-ArgumentCompleter"));
    }

    #[test]
    fn nushell_completion_uses_export_extern() {
        let script = completion_script(ShellKind::Nushell, "forge").unwrap();
        assert!(script.contains("export extern"));
        assert!(script.contains("--scope"));
        assert!(script.contains("--mode"));
    }

    #[test]
    fn elvish_completion_uses_arg_completer() {
        let script = completion_script(ShellKind::Elvish, "forge").unwrap();
        assert!(script.contains("edit:completion:arg-completer[forge]"));
        assert!(script.contains("subcmds"));
    }

    #[test]
    fn tcsh_completion_uses_complete_keyword() {
        let script = completion_script(ShellKind::Tcsh, "forge").unwrap();
        assert!(script.contains("complete forge"));
        assert!(script.contains("'c/memory/"));
    }

    #[test]
    fn cmd_rejects_completion_with_error() {
        let result = completion_script(ShellKind::Cmd, "forge");
        assert!(matches!(result, Err(ShellError::CompletionUnsupported { .. })));
    }

    #[test]
    fn unknown_rejects_completion_with_error() {
        let result = completion_script(ShellKind::Unknown, "forge");
        assert!(matches!(result, Err(ShellError::CompletionUnsupported { .. })));
    }

    #[test]
    fn install_targets_zsh_path() {
        let home = std::path::Path::new("/Users/test");
        let targets = install_targets(ShellKind::Zsh, home, "forge");
        assert_eq!(targets.len(), 1);
        assert!(targets[0].path.contains(".zsh/completions/_forge"));
    }

    #[test]
    fn install_targets_powershell_windows_path() {
        let home = std::path::Path::new("C:\\Users\\test");
        let targets = install_targets(ShellKind::PowerShellWindows, home, "forge");
        assert_eq!(targets.len(), 1);
        assert!(targets[0].path.contains("PowerShell"));
    }

    #[test]
    fn install_targets_cmd_returns_empty() {
        let home = std::path::Path::new("/Users/test");
        let targets = install_targets(ShellKind::Cmd, home, "forge");
        assert!(targets.is_empty());
    }

    #[test]
    fn install_targets_fish_path() {
        let home = std::path::Path::new("/Users/test");
        let targets = install_targets(ShellKind::Fish, home, "forge");
        assert_eq!(targets.len(), 1);
        assert!(targets[0].path.contains(".config/fish/completions/forge.fish"));
    }
}
