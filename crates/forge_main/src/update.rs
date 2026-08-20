use std::sync::Arc;

use colored::Colorize;
use forge_api::API;
use forge_config::{Update, UpdateFrequency};
use forge_select::ForgeWidget;
use forge_tracker::VERSION;
use update_informer::{Check, Version, registry};
use url::Url;

/// Fork-owned release repository. Asset names and download URLs are derived
/// from the release matrix in `.github/workflows/release.yml`; keeping the
/// mapping explicit prevents the updater from guessing an asset for an
/// unsupported platform.
///
/// Centralized in `forge_config` so `forge_services` (heliosdoctor) can
/// report the same channel without depending back on `forge_main`.
pub const DEFAULT_UPDATE_REPO: &str = forge_config::DEFAULT_UPDATE_REPO;

/// Return the asset name for a given target under a specific binary prefix
/// (`forge` vs `helioslite`). Exposed so the binary-aware updater can pull
/// assets that match the running executable, keeping the two identities'
/// self-update channels separate.
fn release_asset_for_target_with_prefix(target: &str, prefix: &str) -> Option<String> {
    let suffix = match target {
        "aarch64-apple-darwin" => "aarch64-apple-darwin",
        "x86_64-apple-darwin" => "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu" => "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu" => "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-musl" => "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-musl" => "x86_64-unknown-linux-musl",
        "aarch64-pc-windows-msvc" => "aarch64-pc-windows-msvc.exe",
        "x86_64-pc-windows-msvc" => "x86_64-pc-windows-msvc.exe",
        _ => return None,
    };
    Some(format!("{prefix}-{suffix}"))
}

/// Returns the asset-name prefix for the currently running binary
/// (`helioslite` for the canonical binary, `forge` for everything else).
/// Driven by `argv[0]`'s file stem, matching the CLI name detection in
/// `forge_main::main`. Delegates to the shared helper in `forge_config`.
pub fn current_binary_prefix() -> &'static str {
    forge_config::ConfigReader::binary_prefix()
}

/// Build the canonical GitHub release URL for a supported target asset.
///
/// Versions may be supplied with or without the conventional leading `v`.
/// The returned URL is only constructed for non-empty, release-safe version
/// strings and targets present in the release matrix.
///
/// Keep alive for platform updaters that pin an exact release; the asset
/// mapping is separately consumed by the Windows updater.
#[allow(dead_code)]
fn release_asset_url(version: &str, target: &str) -> Option<Url> {
    release_asset_url_with_prefix(version, target, current_binary_prefix())
}

fn release_asset_url_with_prefix(version: &str, target: &str, prefix: &str) -> Option<Url> {
    let version = version.strip_prefix('v').unwrap_or(version);
    if version.is_empty()
        || version.starts_with('v')
        || version.len() > 64
        || !version
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_digit())
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return None;
    }
    let asset = release_asset_for_target_with_prefix(target, prefix)?;
    Url::parse(&format!(
        "https://github.com/{DEFAULT_UPDATE_REPO}/releases/download/v{version}/{asset}"
    ))
    .ok()
}

/// Runs the official installation script to update Forge, failing silently.
/// When `auto_update` is true, exits immediately after a successful update
/// without prompting the user.
///
/// Phenotype rename: by default we hit `helioslite.dev/cli`; if that
/// endpoint is unreachable we fall back to the upstream `forgecode.dev/cli`
/// URL so users running pre-rename builds keep working.
async fn execute_update_command(api: Arc<impl API>, auto_update: bool) {
    // The POSIX `curl … | sh` pipe cannot work on native Windows: there is no
    // `sh`, and cmd.exe stops resolving commands once PATH exceeds its ~2047
    // char batch limit. `update_command` returns a native Windows updater there
    // and the platform-appropriate one-liner elsewhere.
    let command = update_command();

    // Spawn a new task that won't block the main application
    let output = api.execute_shell_command_raw(&command).await;

    match output {
        Err(err) => {
            // Send an event to the tracker on failure
            // We don't need to handle this result since we're failing silently
            let _ = send_update_failure_event(&format!("Auto update failed {err}")).await;
        }
        Ok(output) => {
            if output.success() {
                let should_exit = if auto_update {
                    true
                } else {
                    let answer = forge_select::ForgeWidget::confirm(
                        "You need to close forge to complete update. Do you want to close it now?",
                    )
                    .with_default(true)
                    .prompt();
                    answer.unwrap_or_default().unwrap_or_default()
                };
                if should_exit {
                    std::process::exit(0);
                }
            } else {
                let exit_output = match output.code() {
                    Some(code) => format!("Process exited with code: {code}"),
                    None => "Process exited without code".to_string(),
                };
                let _ =
                    send_update_failure_event(&format!("Auto update failed, {exit_output}",)).await;
            }
        }
    }
}

/// Returns the update command for the current platform.
///
/// On Windows this returns a PowerShell invocation that downloads the release
/// binary and stages an atomic swap; on other platforms it returns the official
/// `curl … | sh` one-liner.
fn update_command() -> String {
    #[cfg(windows)]
    {
        windows_update_command().unwrap_or_else(|| "exit 1".to_string())
    }
    #[cfg(not(windows))]
    {
        // Phenotype rename: prefer the renamed endpoint, falling back to the
        // upstream forgecode.dev/cli URL so pre-rename builds keep working.
        let primary = std::env::var("HELIOSLITE_UPDATE_URL")
            .unwrap_or_else(|_| "https://helioslite.dev/cli".to_string());
        let fallback = "https://forgecode.dev/cli";
        format!("(curl -fsSL {primary} || curl -fsSL {fallback}) | sh")
    }
}

/// Builds a native Windows update command.
///
/// The running `forge.exe` (or `helioslite.exe`) is locked while the process
/// is alive, so the new binary cannot be replaced in place. Instead we:
///
/// 1. Download `{prefix}-{arch}-pc-windows-msvc.exe` to `<binary>.new` next to
///    the current binary using PowerShell (absolute paths only, immune to the
///    length-capped PATH that breaks `curl` resolution in cmd.exe).
/// 2. Stage a small `.cmd` helper that waits for the running exe to exit, swaps
///    `<binary>.new` over `<binary>`, cleans up, and relaunches.
/// 3. Launch that helper detached, so it survives the binary exiting.
///
/// The prefix and install directory are derived from the running binary's
/// file stem so `helioslite.exe` self-updates from `helioslite-*.exe` assets
/// under `%LOCALAPPDATA%\Programs\heliosLite\`, while `forge.exe` keeps its
/// historical `%LOCALAPPDATA%\Programs\Forge\` + `forge-*.exe` layout. This
/// keeps the two identities' self-update channels separate.
#[cfg(windows)]
fn windows_update_command() -> Option<String> {
    use std::io::Write;

    // Resolve the release asset name from the release matrix. This fails
    // closed for any target the release workflow does not publish, rather
    // than guessing an asset name inside PowerShell.
    let target = if cfg!(target_arch = "aarch64") {
        "aarch64-pc-windows-msvc"
    } else {
        "x86_64-pc-windows-msvc"
    };
    let prefix = current_binary_prefix();
    let asset = release_asset_for_target_with_prefix(target, prefix)?;

    // Install dir / file names mirror argv[0]'s file stem so the two
    // identities never collide on the same on-disk path.
    let binary_stem = match prefix {
        "helioslite" => "helioslite",
        _ => "forge",
    };
    let install_dir_name = match prefix {
        "helioslite" => "heliosLite",
        _ => "Forge",
    };
    let swap_marker_basename = match prefix {
        "helioslite" => "_helioslite_swap",
        _ => "_forge_swap",
    };

    let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
    let install_dir = format!(r"{local_app_data}\Programs\{install_dir_name}");
    let exe_name = format!("{binary_stem}.exe");
    let new_exe = format!(r"{install_dir}\{exe_name}.new");
    let exe = format!(r"{install_dir}\{exe_name}");
    let swap_bat = format!(r"{install_dir}\{swap_marker_basename}.cmd");
    let ps_path = format!(r"{install_dir}\{binary_stem}-update.ps1");

    // The wait loop uses full paths to tasklist/find/timeout so it keeps
    // working even when the inherited PATH is polluted beyond cmd.exe's
    // ~2047-char batch limit. move/del/start are cmd built-ins. The
    // `IMAGENAME eq` filter targets the binary we're replacing.
    let swap_content = format!(
        "@echo off\r\n\
         set /a count=0\r\n\
         :wait\r\n\
         set /a count+=1\r\n\
         if %count% gtr 900 goto abort\r\n\
         %SystemRoot%\\System32\\tasklist.exe /FI \"IMAGENAME eq {exe_name}\" 2>nul | %SystemRoot%\\System32\\find.exe /I \"{exe_name}\" >nul\r\n\
         if not errorlevel 1 (\r\n\
           %SystemRoot%\\System32\\timeout.exe /t 1 /nobreak >nul\r\n\
           goto wait\r\n\
         )\r\n\
         move /Y \"{new_exe}\" \"{exe}\"\r\n\
         del \"%~f0\"\r\n\
         start \"\" \"{exe}\"\r\n\
         exit /b 0\r\n\
         :abort\r\n\
         del \"%~f0\"\r\n\
         del \"{new_exe}\"\r\n\
         exit /b 1\r\n"
    );

    std::fs::create_dir_all(&install_dir).ok()?;
    let mut swap_file = std::fs::File::create(&swap_bat).ok()?;
    swap_file.write_all(swap_content.as_bytes()).ok()?;

    let ps_script = format!(
        r#"$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\{install_dir_name}'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$repo = if ($env:HELIOSLITE_REPO) {{ $env:HELIOSLITE_REPO }} else {{ 'KooshaPari/forgecode' }}
$url = 'https://github.com/' + $repo + '/releases/latest/download/{asset}'
$new = Join-Path $dir ('{exe_name}' + '.new')
Invoke-WebRequest -Uri $url -OutFile $new -UseBasicParsing
$swap = Join-Path $dir ('{swap_marker_basename}' + '.cmd')
Start-Process -FilePath $swap -WindowStyle Hidden
:"#,
        asset = asset,
        exe_name = exe_name,
        install_dir_name = install_dir_name,
        swap_marker_basename = swap_marker_basename,
    );

    let mut ps_file = std::fs::File::create(&ps_path).ok()?;
    ps_file.write_all(ps_script.as_bytes()).ok()?;

    Some(format!(
        r#"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe -NoProfile -ExecutionPolicy Bypass -File "{ps_path}""#
    ))
}
async fn confirm_update(version: Version) -> bool {
    let answer = ForgeWidget::confirm(format!(
        "Confirm upgrade from {} -> {} (latest)?",
        VERSION.to_string().bold().white(),
        version.to_string().bold().white()
    ))
    .with_default(true)
    .prompt();

    match answer {
        Ok(Some(result)) => result,
        Ok(None) => false, // User canceled
        Err(_) => false,   // Error occurred
    }
}

fn should_check_for_updates(frequency: &UpdateFrequency) -> bool {
    !matches!(frequency, UpdateFrequency::Never)
}

// Phenotype-org: detect non-interactive (agent/CI) invocations to skip the
// update check entirely.  Avoids a ~220ms GitHub API round-trip on every
// agent spawn; see profiling notes in perf/profile-zig-hotpath-2026-06-30.
fn is_non_interactive() -> bool {
    use std::io::IsTerminal;
    // CI env vars (standard subset)
    if std::env::var_os("CI").is_some()
        || std::env::var_os("FORGE_NON_INTERACTIVE").is_some()
        || std::env::var_os("FORGE_AGENT_MODE").is_some()
    {
        return true;
    }
    // stdin is not a TTY — running in a pipe or scripted context
    !std::io::stdin().is_terminal()
}

/// Checks if there is an update available
pub async fn on_update(api: Arc<impl API>, update: Option<&Update>) {
    let update = update.cloned().unwrap_or_default();
    let frequency = update.frequency.unwrap_or_default();

    if !should_check_for_updates(&frequency) {
        return;
    }

    // Phenotype-org: skip update check in CI / non-TTY / agent-batch mode.
    // Each forge process pays ~220ms for a GitHub API call when `frequency`
    // is `Always`; agent fleets spawn many short-lived processes and this
    // dominates per-invocation overhead.
    if is_non_interactive() {
        return;
    }

    let auto_update = update.auto_update.unwrap_or_default();

    // Check if version is development version, in which case we skip the update
    // check
    if VERSION.contains("dev") || VERSION == "0.1.0" {
        // Skip update for development version 0.1.0
        return;
    }

    // Phenotype rename: prefer the renamed-binary GitHub repo
    // (`KooshaPari/heliosLite`). In flight, the `KooshaPari/forgecode` releases
    // are kept as the canonical source for both name chains; `HELIOSLITE_REPO`
    // overrides the lookup so nightlies can target a third-party fork without
    // recompiling.
    //
    // Tombstone: until the rename is pushed to remote (Gate 4b),
    // `KooshaPari/heliosLite` doesn't exist and the lookup 404s. We swallow
    // that case and try the legacy `KooshaPari/forgecode` releases so users on
    // pre-rename builds keep getting notified. This branch will be removed once
    // the rename is permanent.
    let primary_repo =
        std::env::var("HELIOSLITE_REPO").unwrap_or_else(|_| "KooshaPari/heliosLite".to_string());
    let legacy_repo = "KooshaPari/forgecode";
    let informer_primary = update_informer::new(registry::GitHub, primary_repo.as_str(), VERSION)
        .interval(frequency.clone().into());
    let informer_legacy =
        update_informer::new(registry::GitHub, legacy_repo, VERSION).interval(frequency.into());

    if let Some(version) = informer_primary
        .check_version()
        .ok()
        .flatten()
        .or_else(|| informer_legacy.check_version().ok().flatten())
        && (auto_update || confirm_update(version).await)
    {
        execute_update_command(api, auto_update).await;
    }
}

/// Sends an event to the tracker when an update fails
async fn send_update_failure_event(error_msg: &str) -> anyhow::Result<()> {
    tracing::error!(error = error_msg, "Update failed");
    // Always return Ok since we want to fail silently
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_should_skip_update_check_when_frequency_is_never() {
        let fixture = UpdateFrequency::Never;

        let actual = should_check_for_updates(&fixture);

        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn release_asset_maps_supported_target_triples_for_forge_prefix() {
        for (target, expected) in [
            ("aarch64-apple-darwin", "forge-aarch64-apple-darwin"),
            ("x86_64-apple-darwin", "forge-x86_64-apple-darwin"),
            (
                "aarch64-unknown-linux-gnu",
                "forge-aarch64-unknown-linux-gnu",
            ),
            ("x86_64-unknown-linux-gnu", "forge-x86_64-unknown-linux-gnu"),
            (
                "aarch64-unknown-linux-musl",
                "forge-aarch64-unknown-linux-musl",
            ),
            (
                "x86_64-unknown-linux-musl",
                "forge-x86_64-unknown-linux-musl",
            ),
            (
                "aarch64-pc-windows-msvc",
                "forge-aarch64-pc-windows-msvc.exe",
            ),
            ("x86_64-pc-windows-msvc", "forge-x86_64-pc-windows-msvc.exe"),
        ] {
            assert_eq!(
                release_asset_for_target_with_prefix(target, "forge").as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn release_asset_maps_supported_target_triples_for_helioslite_prefix() {
        for (target, expected) in [
            ("aarch64-apple-darwin", "helioslite-aarch64-apple-darwin"),
            ("x86_64-apple-darwin", "helioslite-x86_64-apple-darwin"),
            (
                "aarch64-pc-windows-msvc",
                "helioslite-aarch64-pc-windows-msvc.exe",
            ),
            (
                "x86_64-pc-windows-msvc",
                "helioslite-x86_64-pc-windows-msvc.exe",
            ),
        ] {
            assert_eq!(
                release_asset_for_target_with_prefix(target, "helioslite").as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn release_asset_rejects_unsupported_target_triples() {
        for target in [
            "aarch64-linux-android",
            "x86_64-pc-windows-gnu",
            "x86_64-unknown-freebsd",
            "wasm32-unknown-unknown",
            "",
        ] {
            assert!(
                release_asset_for_target_with_prefix(target, "forge").is_none(),
                "accepted {target:?}"
            );
        }
    }

    #[test]
    fn release_asset_url_uses_helioslite_prefix_for_helioslite_binary() {
        assert_eq!(
            release_asset_url_with_prefix("2.10.2", "aarch64-apple-darwin", "helioslite")
                .unwrap()
                .as_str(),
            "https://github.com/KooshaPari/forgecode/releases/download/v2.10.2/helioslite-aarch64-apple-darwin"
        );
        assert_eq!(
            release_asset_url_with_prefix("v2.10.2", "x86_64-pc-windows-msvc", "helioslite",)
                .unwrap()
                .as_str(),
            "https://github.com/KooshaPari/forgecode/releases/download/v2.10.2/helioslite-x86_64-pc-windows-msvc.exe"
        );
    }

    #[test]
    fn release_asset_url_normalizes_version_and_target() {
        assert_eq!(
            release_asset_url_with_prefix("2.10.2", "aarch64-apple-darwin", "forge")
                .unwrap()
                .as_str(),
            "https://github.com/KooshaPari/forgecode/releases/download/v2.10.2/forge-aarch64-apple-darwin"
        );
        assert_eq!(
            release_asset_url_with_prefix("v2.10.2", "x86_64-pc-windows-msvc", "forge",)
                .unwrap()
                .as_str(),
            "https://github.com/KooshaPari/forgecode/releases/download/v2.10.2/forge-x86_64-pc-windows-msvc.exe"
        );
    }

    #[test]
    fn release_asset_url_rejects_invalid_versions_and_targets() {
        for version in [
            "",
            "v",
            "vv2.10.2",
            "release-2.10.2",
            "2.10.2/../../x",
            "2.10.2?download=1",
            "2.10.2#x",
        ] {
            assert!(
                release_asset_url_with_prefix(version, "aarch64-apple-darwin", "forge").is_none()
            );
        }
        assert!(
            release_asset_url_with_prefix("2.10.2", "x86_64-pc-windows-gnu", "forge").is_none()
        );
    }
}
