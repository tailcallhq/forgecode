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
/// On Windows, the `helioslite_helper.exe` sibling binary is the primary
/// update mechanism (download → SHA-256 verify → wait → atomic swap →
/// relaunch, all in one detached process). The PowerShell scaffolder
/// (`windows_update_command`) remains as a fallback for installations that
/// predate the helper binary. On other platforms the POSIX `curl … | sh`
/// one-liner is used.
fn update_command() -> String {
    #[cfg(windows)]
    {
        windows_update_with_helper()
            .or_else(windows_update_command)
            .unwrap_or_else(|| "exit 1".to_string())
    }
    #[cfg(not(windows))]
    {
        // Both URLs serve the same installer script from our own
        // infrastructure (helioslite.dev is the canonical, forgecode.dev
        // is the legacy pre-rename URL we still maintain for older
        // clients). The fallback is plain CDN redundancy, not a switch
        // to a different code source.
        let primary = std::env::var("HELIOSLITE_UPDATE_URL")
            .unwrap_or_else(|_| "https://helioslite.dev/cli".to_string());
        let fallback = "https://forgecode.dev/cli";
        format!("(curl -fsSL {primary} || curl -fsSL {fallback}) | sh")
    }
}

/// Primary Windows update path: spawn the `helioslite_helper.exe` sibling
/// binary which handles the entire download → verify → wait → swap → relaunch
/// flow in one detached process. Returns `None` if the helper isn't
/// present alongside the running exe (e.g. older installs that predate the
/// helper binary), in which case the caller falls back to
/// `windows_update_command`.
#[cfg(windows)]
fn windows_update_with_helper() -> Option<String> {
    use std::io::Write;

    // Mirror the path resolution in `windows_update_command` so the asset
    // name, install dir, and target exe paths stay consistent between the
    // two code paths.
    let target = if cfg!(target_arch = "aarch64") {
        "aarch64-pc-windows-msvc"
    } else {
        "x86_64-pc-windows-msvc"
    };
    let prefix = current_binary_prefix();
    let asset = release_asset_for_target_with_prefix(target, prefix)?;

    let binary_stem = match prefix {
        "helioslite" => "helioslite",
        _ => "forge",
    };
    let install_dir_name = match prefix {
        "helioslite" => "heliosLite",
        _ => "Forge",
    };

    let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
    let install_dir = format!(r"{local_app_data}\Programs\{install_dir_name}");
    let exe_name = format!("{binary_stem}.exe");
    let new_exe = format!(r"{install_dir}\{exe_name}.new");
    let exe = format!(r"{install_dir}\{exe_name}");
    let helper_exe = format!(r"{install_dir}\helioslite_helper.exe");

    // Bail out so the PS1 scaffolder can run if the helper isn't shipped.
    // This is the common case for any install that predates the helper
    // landing in the release archive.
    if !std::path::Path::new(&helper_exe).exists() {
        return None;
    }

    let release_repo =
        std::env::var("HELIOSLITE_REPO").unwrap_or_else(|_| DEFAULT_UPDATE_REPO.to_string());
    let parent_pid = std::process::id();

    // Drive the helper detached so it outlives us. Stdin/stdout/stderr are
    // closed so closing the parent terminal doesn't propagate to the helper.
    let log_path = format!(r"{install_dir}\helioslite-update.log");
    // `CREATE_NO_WINDOW` (0x08000000) so the helper doesn't pop a console.
    // `DETACHED_PROCESS` (0x00000008) so closing our terminal doesn't
    // propagate a kill to the helper mid-swap.
    const FLAGS: u32 = 0x08000000 | 0x00000008;
    let mut cmd = std::process::Command::new(&helper_exe);
    cmd.args([
        "download",
        &release_repo,
        &asset,
        "wait",
        &parent_pid.to_string(),
        "swap",
        &new_exe,
        &exe,
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    // Detach on Windows by passing CREATE_NO_WINDOW + DETACHED_PROCESS via
    // CommandExt::creation_flags.  We need to reach for windows-sys here.
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(FLAGS);
    }
    match cmd.spawn() {
        Ok(_) => {
            // Best-effort log note: the helper writes its own per-step log to
            // stderr-equivalent, but operators want a "spawned OK" trail too.
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                let _ = writeln!(
                    f,
                    "{}  repo={}  asset={}  helper=spawned pid_parent={}",
                    chrono::Utc::now().to_rfc3339(),
                    release_repo,
                    asset,
                    parent_pid,
                );
            }
            // Returning "exit 0" here signals "we handed off the update,
            // don't try anything else".  The caller will spawn this string
            // via execute_shell_command_raw which will short-circuit on
            // success; the helper does the real work in the background.
            Some("exit 0".to_string())
        }
        Err(e) => {
            // Helper refused to spawn. Fall through to PS1 scaffolder by
            // returning None.
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                let _ = writeln!(
                    f,
                    "{}  repo={}  helper spawn failed: {e}; falling back to PS1",
                    chrono::Utc::now().to_rfc3339(),
                    release_repo,
                );
            }
            None
        }
    }
}

/// Fallback Windows update path: generate a small PowerShell downloader +
/// `.cmd` swap script on disk and run it via `powershell.exe`. Kept so
/// installs that predate the `helioslite_helper.exe` binary landing in the
/// release archive still self-update. New installs (or any install where
/// the helper is present) skip this entirely via `windows_update_with_helper`.
///
/// The PowerShell scaffolder is retained only because removing it outright
/// would brick the upgrade path for anyone running a build older than the
/// first release that ships the helper. Once every supported build has the
/// helper in its install dir for ≥2 minor releases, this function can be
/// deleted along with the `_forge_swap.cmd` / `forge-update.ps1` files it
/// produces.
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

    // Repo mapping: the HeliosLite fork (the package you are running) lives
    // at `KooshaPari/forgecode` on GitHub. The "helioslite" binary name and
    // the "forgecode" repo name are not the same axis; the binary is the
    // Phenotype rename of upstream tailcallhq/forgecode, and the repo is the
    // canonical fork we ship from. `HELIOSLITE_REPO` overrides the lookup
    // for nightlies or third-party forks.
    //
    // NO FALLBACK POLICY: if the configured repo is unreachable or has no
    // release, we do NOT silently switch to upstream tailcallhq/forgecode
    // or to the previously-suggested `KooshaPari/heliosLite` (which doesn't
    // exist). The informer reports the version; if it's `None` we stay
    // quiet. The actual download path (helper / PS1 scaffolder) does the
    // same: log a one-line reason and stay on the current binary.
    let repo = std::env::var("HELIOSLITE_REPO").unwrap_or_else(|_| DEFAULT_UPDATE_REPO.to_string());
    let informer =
        update_informer::new(registry::GitHub, repo.as_str(), VERSION).interval(frequency.into());

    if let Some(version) = informer.check_version().ok().flatten()
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
            "2.10.2#frag",
        ] {
            assert!(
                release_asset_url_with_prefix(version, "aarch64-apple-darwin", "forge").is_none()
            );
        }
        assert!(
            release_asset_url_with_prefix("2.10.2", "x86_64-pc-windows-gnu", "forge").is_none()
        );
    }

    /// Pins the default repo to `KooshaPari/forgecode`.  HeliosLite's live
    /// releases ship from there; the upstream `tailcallhq/forgecode` and the
    /// previously-suggested `KooshaPari/heliosLite` (which doesn't exist) are
    /// never auto-fallbacks.  Override via $env:HELIOSLITE_REPO at runtime.
    /// If this test breaks, the in-app informer (and the helper binary's
    /// download URL by extension) is silently pointing at the wrong repo.
    #[test]
    fn default_update_repo_resolves_to_kooshapari_forgecode() {
        assert_eq!(DEFAULT_UPDATE_REPO, "KooshaPari/forgecode");
        // The helper-driven download URL must not include the phantom
        // `KooshaPari/heliosLite` repo anywhere in its composition.
        let url = release_asset_url_with_prefix(
            "9.9.9",
            "x86_64-pc-windows-msvc",
            current_binary_prefix(),
        )
        .expect("Windows target must produce a URL")
        .to_string();
        assert!(
            url.starts_with("https://github.com/KooshaPari/forgecode/"),
            "default update URL must target KooshaPari/forgecode; got {url}"
        );
        assert!(
            !url.contains("KooshaPari/heliosLite"),
            "default update URL must not reference the non-existent KooshaPari/heliosLite; got {url}"
        );
        assert!(
            !url.contains("tailcallhq/forgecode"),
            "default update URL must not target upstream tailcallhq; got {url}"
        );
    }
}
