use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Result};
use include_dir::{Dir, include_dir};

/// Embeds shell plugin files for fish integration
static FISH_PLUGIN_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../shell-plugin/fish");

/// Generates the complete fish plugin by combining embedded files.
///
/// Iterates through all embedded files in `shell-plugin/fish/`, stripping
/// comments and empty lines, then appends a `_FORGE_PLUGIN_LOADED` marker.
pub fn generate_fish_plugin() -> Result<String> {
    let mut output = String::new();

    for file in forge_embed::files(&FISH_PLUGIN_DIR) {
        let content = super::super::zsh::normalize_script(std::str::from_utf8(file.contents())?);
        for line in content.lines() {
            let trimmed = line.trim();
            // Skip empty lines and comment lines
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                output.push_str(line);
                output.push('\n');
            }
        }
    }

    // Set environment variable to indicate plugin is loaded (with timestamp)
    output.push_str("\nset -g _FORGE_PLUGIN_LOADED (date +%s)\n");

    Ok(output)
}

/// Generates the Fish theme for Forge.
///
/// Returns the content of `shell-plugin/fish/theme.fish` with a
/// `_FORGE_THEME_LOADED` marker appended.
pub fn generate_fish_theme() -> Result<String> {
    let mut content = super::super::zsh::normalize_script(include_str!(
        "../../../../shell-plugin/fish/theme.fish"
    ));

    // Set environment variable to indicate theme is loaded (with timestamp)
    content.push_str("\nset -g _FORGE_THEME_LOADED (date +%s)\n");

    Ok(content)
}

/// Executes a Fish script with streaming output.
///
/// # Arguments
///
/// * `script_content` - The Fish script content to execute
/// * `script_name` - Descriptive name for the script (used in error messages)
///
/// # Errors
///
/// Returns error if the script cannot be executed, if output streaming fails,
/// or if the script exits with a non-zero status code
fn execute_fish_script_with_streaming(script_content: &str, script_name: &str) -> Result<()> {
    let script_content = super::super::zsh::normalize_script(script_content);

    let mut child = std::process::Command::new("fish")
        .arg("-c")
        .arg(&script_content)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context(format!("Failed to execute fish {script_name} script"))?;

    // Get stdout and stderr handles
    let stdout = child.stdout.take().context("Failed to capture stdout")?;
    let stderr = child.stderr.take().context("Failed to capture stderr")?;

    // Use scoped threads for safer streaming with automatic joining
    std::thread::scope(|s| {
        // Stream stdout line by line
        s.spawn(|| {
            let stdout_reader = BufReader::new(stdout);
            for line in stdout_reader.lines() {
                match line {
                    Ok(line) => println!("{line}"),
                    Err(e) => eprintln!("Error reading stdout: {e}"),
                }
            }
        });

        // Stream stderr line by line
        s.spawn(|| {
            let stderr_reader = BufReader::new(stderr);
            for line in stderr_reader.lines() {
                match line {
                    Ok(line) => eprintln!("{line}"),
                    Err(e) => eprintln!("Error reading stderr: {e}"),
                }
            }
        });
    });

    // Wait for the child process to complete
    let status = child
        .wait()
        .context(format!("Failed to wait for fish {script_name} script"))?;

    if !status.success() {
        let exit_code = status
            .code()
            .map_or_else(|| "unknown".to_string(), |code| code.to_string());

        anyhow::bail!("Fish {script_name} script failed with exit code: {exit_code}");
    }

    Ok(())
}

/// Runs diagnostics on the Fish shell environment with streaming output.
///
/// # Errors
///
/// Returns error if the doctor script cannot be executed
pub fn run_fish_doctor() -> Result<()> {
    let script_content = include_str!("../../../../shell-plugin/doctor.fish");
    execute_fish_script_with_streaming(script_content, "doctor")
}

/// Result of Fish setup operation
#[derive(Debug)]
pub struct FishSetupResult {
    /// Status message describing what was done
    pub message: String,
    /// Path to backup file if one was created
    pub backup_path: Option<PathBuf>,
}

/// Sets up Fish integration by updating `config.fish` with plugin and theme.
///
/// # Arguments
///
/// * `disable_nerd_font` - If true, adds `NERD_FONT=0` to config.fish
/// * `forge_editor` - If Some(editor), adds `FORGE_EDITOR` export to config.fish
///
/// # Errors
///
/// Returns error if:
/// - The HOME environment variable is not set
/// - The config.fish file cannot be read or written
/// - Invalid forge markers are found (incomplete or incorrectly ordered)
/// - A backup of the existing config.fish cannot be created
pub fn setup_fish_integration(
    disable_nerd_font: bool,
    forge_editor: Option<&str>,
) -> Result<FishSetupResult> {
    const START_MARKER: &str = "# >>> forge initialize >>>";
    const END_MARKER: &str = "# <<< forge initialize <<<";
    const FORGE_INIT_CONFIG_RAW: &str = include_str!("../../../../shell-plugin/forge.setup.fish");
    let forge_init_config = super::super::zsh::normalize_script(FORGE_INIT_CONFIG_RAW);

    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let xdg_config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    let fish_config_dir = PathBuf::from(&xdg_config).join("fish");
    let config_fish_path = fish_config_dir.join("config.fish");

    // Ensure the fish config directory exists
    if !fish_config_dir.exists() {
        fs::create_dir_all(&fish_config_dir).context(format!(
            "Failed to create fish config directory {}",
            fish_config_dir.display()
        ))?;
    }

    // Read existing config.fish or create new one
    let content = if config_fish_path.exists() {
        fs::read_to_string(&config_fish_path)
            .context(format!("Failed to read {}", config_fish_path.display()))?
    } else {
        String::new()
    };

    let mut lines: Vec<String> = content.lines().map(String::from).collect();

    // Parse markers to determine their state
    let start_idx = lines.iter().position(|line| line.trim() == START_MARKER);
    let end_idx = lines.iter().position(|line| line.trim() == END_MARKER);

    // Build the forge config block with markers
    let mut forge_config: Vec<String> = vec![START_MARKER.to_string()];
    forge_config.extend(forge_init_config.lines().map(String::from));

    // Add nerd font configuration if requested
    if disable_nerd_font {
        forge_config.push(String::new());
        forge_config.push(
            "# Disable Nerd Fonts (set during setup - icons not displaying correctly)".to_string(),
        );
        forge_config.push(
            "# To re-enable: remove this line and install a Nerd Font from https://www.nerdfonts.com/"
                .to_string(),
        );
        forge_config.push("set -gx NERD_FONT 0".to_string());
    }

    // Add editor configuration if requested
    if let Some(editor) = forge_editor {
        forge_config.push(String::new());
        forge_config.push("# Editor for editing prompts (set during setup)".to_string());
        forge_config.push("# To change: update FORGE_EDITOR or remove to use $EDITOR".to_string());
        forge_config.push(format!("set -gx FORGE_EDITOR \"{editor}\""));
    }

    forge_config.push(END_MARKER.to_string());

    // Determine action based on marker state
    let (new_content, config_action) = match (start_idx, end_idx) {
        (Some(start), Some(end)) if start < end => {
            // Markers exist - replace content between them
            lines.splice(start..=end, forge_config.iter().cloned());
            (lines.join("\n") + "\n", "updated")
        }
        (Some(_), Some(_)) | (Some(_), None) | (None, Some(_)) => {
            anyhow::bail!(
                "Invalid forge markers found in {}",
                config_fish_path.display()
            );
        }
        (None, None) => {
            // No markers - add them at the end
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.extend(forge_config.iter().cloned());
            (lines.join("\n") + "\n", "added")
        }
    };

    // Create backup of existing config.fish if it exists
    let backup_path = if config_fish_path.exists() {
        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let parent = config_fish_path
            .parent()
            .context("config.fish path has no parent directory")?;
        let filename = config_fish_path
            .file_name()
            .context("config.fish path has no filename")?;
        let filename_str = filename.to_str().context("filename is not valid UTF-8")?;

        let backup = parent.join(format!("{filename_str}.bak.{timestamp}"));
        fs::copy(&config_fish_path, &backup)
            .context(format!("Failed to create backup at {}", backup.display()))?;
        Some(backup)
    } else {
        None
    };

    // Write back to config.fish
    fs::write(&config_fish_path, &new_content)
        .context(format!("Failed to write to {}", config_fish_path.display()))?;

    Ok(FishSetupResult {
        message: format!("forge fish plugins {config_action}"),
        backup_path,
    })
}

/// Result of a teardown operation.
#[derive(Debug)]
pub struct FishTeardownResult {
    /// Status message describing what was done.
    pub message: String,
    /// Path to backup file if one was created.
    pub backup_path: Option<PathBuf>,
}

/// Teardowns Fish integration by removing the forge block from config.fish.
///
/// Finds the `# >>> forge initialize >>>` / `# <<< forge initialize <<<`
/// markers and removes everything between them (inclusive). Creates a backup
/// before modifying the file.
///
/// # Errors
///
/// Returns error if:
/// - The HOME environment variable is not set
/// - The config.fish file cannot be read or written
/// - No forge markers are found
/// - A backup of the existing config.fish cannot be created
pub fn teardown_fish_integration() -> Result<FishTeardownResult> {
    const START_MARKER: &str = "# >>> forge initialize >>>";
    const END_MARKER: &str = "# <<< forge initialize <<<";

    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let xdg_config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    let config_fish_path = PathBuf::from(&xdg_config).join("fish/config.fish");

    if !config_fish_path.exists() {
        anyhow::bail!("{} not found", config_fish_path.display());
    }

    let content = fs::read_to_string(&config_fish_path)
        .context(format!("Failed to read {}", config_fish_path.display()))?;

    let mut lines: Vec<String> = content.lines().map(String::from).collect();

    let start_idx = lines.iter().position(|line| line.trim() == START_MARKER);
    let end_idx = lines.iter().position(|line| line.trim() == END_MARKER);

    let (start, end) = match (start_idx, end_idx) {
        (Some(s), Some(e)) if s < e => (s, e),
        (None, None) => {
            anyhow::bail!(
                "No forge markers found in {}. Nothing to teardown.",
                config_fish_path.display()
            );
        }
        _ => {
            anyhow::bail!(
                "Invalid forge markers found in {}. Please fix manually.",
                config_fish_path.display()
            );
        }
    };

    // Create backup before modifying
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let parent = config_fish_path
        .parent()
        .context("config.fish path has no parent directory")?;
    let filename = config_fish_path
        .file_name()
        .context("config.fish path has no filename")?;
    let filename_str = filename.to_str().context("filename is not valid UTF-8")?;
    let backup = parent.join(format!("{filename_str}.bak.{timestamp}"));
    fs::copy(&config_fish_path, &backup)
        .context(format!("Failed to create backup at {}", backup.display()))?;

    // Remove the marker block and any trailing blank line
    let remove_end = if end + 1 < lines.len() && lines[end + 1].trim().is_empty() {
        end + 1
    } else {
        end
    };
    // Also remove a leading blank line if present
    let remove_start = if start > 0 && lines[start - 1].trim().is_empty() {
        start - 1
    } else {
        start
    };
    lines.drain(remove_start..=remove_end);

    let new_content = lines.join("\n") + "\n";
    fs::write(&config_fish_path, &new_content)
        .context(format!("Failed to write to {}", config_fish_path.display()))?;

    Ok(FishTeardownResult {
        message: "forge fish plugins removed".to_string(),
        backup_path: Some(backup),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{LazyLock, Mutex};

    use super::*;

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn test_setup_fish_integration_creates_config() {
        use tempfile::TempDir;

        let _guard = ENV_LOCK.lock().unwrap();

        let temp_dir = TempDir::new().unwrap();
        let xdg_config = temp_dir.path().join(".config");
        let config_fish_path = xdg_config.join("fish/config.fish");

        let original_home = std::env::var("HOME").ok();
        let original_xdg = std::env::var("XDG_CONFIG_HOME").ok();

        // SAFETY: We hold ENV_LOCK to prevent concurrent environment modifications
        unsafe {
            std::env::set_var("HOME", temp_dir.path());
            std::env::set_var("XDG_CONFIG_HOME", &xdg_config);
        }

        let actual = setup_fish_integration(false, None);

        // SAFETY: We hold ENV_LOCK to prevent concurrent environment modifications
        unsafe {
            if let Some(home) = original_home {
                std::env::set_var("HOME", home);
            } else {
                std::env::remove_var("HOME");
            }
            if let Some(xdg) = original_xdg {
                std::env::set_var("XDG_CONFIG_HOME", xdg);
            } else {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
        }

        assert!(actual.is_ok(), "Setup should succeed: {:?}", actual);

        // The config.fish should be at XDG_CONFIG_HOME/fish/config.fish
        assert!(
            config_fish_path.exists(),
            "config.fish should be created at {:?}",
            config_fish_path
        );
        let content = fs::read_to_string(&config_fish_path).unwrap();
        assert!(content.contains("# >>> forge initialize >>>"));
        assert!(content.contains("# <<< forge initialize <<<"));
        assert!(!content.contains("NERD_FONT"));
    }

    #[test]
    fn test_setup_fish_integration_with_nerd_font_disabled() {
        use tempfile::TempDir;

        let _guard = ENV_LOCK.lock().unwrap();

        let temp_dir = TempDir::new().unwrap();
        let xdg_config = temp_dir.path().join(".config");

        let original_home = std::env::var("HOME").ok();
        let original_xdg = std::env::var("XDG_CONFIG_HOME").ok();

        // SAFETY: We hold ENV_LOCK to prevent concurrent environment modifications
        unsafe {
            std::env::set_var("HOME", temp_dir.path());
            std::env::set_var("XDG_CONFIG_HOME", &xdg_config);
        }

        let actual = setup_fish_integration(true, None);
        assert!(actual.is_ok(), "Setup should succeed: {:?}", actual);

        let config_path = xdg_config.join("fish/config.fish");
        let content = fs::read_to_string(&config_path).unwrap();

        assert!(
            content.contains("set -gx NERD_FONT 0"),
            "Content should contain NERD_FONT=0:\n{}",
            content
        );

        // SAFETY: We hold ENV_LOCK to prevent concurrent environment modifications
        unsafe {
            if let Some(home) = original_home {
                std::env::set_var("HOME", home);
            } else {
                std::env::remove_var("HOME");
            }
            if let Some(xdg) = original_xdg {
                std::env::set_var("XDG_CONFIG_HOME", xdg);
            } else {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
        }
    }

    #[test]
    fn test_setup_fish_integration_with_editor() {
        use tempfile::TempDir;

        let _guard = ENV_LOCK.lock().unwrap();

        let temp_dir = TempDir::new().unwrap();
        let xdg_config = temp_dir.path().join(".config");

        let original_home = std::env::var("HOME").ok();
        let original_xdg = std::env::var("XDG_CONFIG_HOME").ok();

        // SAFETY: We hold ENV_LOCK to prevent concurrent environment modifications
        unsafe {
            std::env::set_var("HOME", temp_dir.path());
            std::env::set_var("XDG_CONFIG_HOME", &xdg_config);
        }

        let actual = setup_fish_integration(false, Some("code --wait"));
        assert!(actual.is_ok(), "Setup should succeed: {:?}", actual);

        let config_path = xdg_config.join("fish/config.fish");
        let content = fs::read_to_string(&config_path).unwrap();

        assert!(
            content.contains("set -gx FORGE_EDITOR \"code --wait\""),
            "Content should contain FORGE_EDITOR:\n{}",
            content
        );

        // SAFETY: We hold ENV_LOCK to prevent concurrent environment modifications
        unsafe {
            if let Some(home) = original_home {
                std::env::set_var("HOME", home);
            } else {
                std::env::remove_var("HOME");
            }
            if let Some(xdg) = original_xdg {
                std::env::set_var("XDG_CONFIG_HOME", xdg);
            } else {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
        }
    }

    #[test]
    fn test_setup_fish_integration_updates_existing_markers() {
        use tempfile::TempDir;

        let _guard = ENV_LOCK.lock().unwrap();

        let temp_dir = TempDir::new().unwrap();
        let xdg_config = temp_dir.path().join(".config");

        let original_home = std::env::var("HOME").ok();
        let original_xdg = std::env::var("XDG_CONFIG_HOME").ok();

        // SAFETY: We hold ENV_LOCK to prevent concurrent environment modifications
        unsafe {
            std::env::set_var("HOME", temp_dir.path());
            std::env::set_var("XDG_CONFIG_HOME", &xdg_config);
        }

        // First setup - with nerd font disabled
        let result = setup_fish_integration(true, None);
        assert!(result.is_ok(), "Initial setup should succeed: {:?}", result);
        assert!(
            result.as_ref().unwrap().backup_path.is_none(),
            "Should not create backup on initial setup"
        );

        let config_path = xdg_config.join("fish/config.fish");
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("set -gx NERD_FONT 0"));
        assert!(!content.contains("FORGE_EDITOR"));

        // Second setup - without nerd font but with editor
        let result = setup_fish_integration(false, Some("nvim"));
        assert!(result.is_ok(), "Update setup should succeed: {:?}", result);

        // Second setup should create a backup
        assert!(result.as_ref().unwrap().backup_path.is_some());

        let content = fs::read_to_string(&config_path).unwrap();
        assert!(
            !content.contains("NERD_FONT 0"),
            "Should not contain NERD_FONT=0 after update:\n{}",
            content
        );
        assert!(
            content.contains("set -gx FORGE_EDITOR \"nvim\""),
            "Should contain FORGE_EDITOR after update:\n{}",
            content
        );

        // Should still have markers and only one set
        assert!(content.contains("# >>> forge initialize >>>"));
        assert!(content.contains("# <<< forge initialize <<<"));
        assert_eq!(
            content.matches("# >>> forge initialize >>>").count(),
            1,
            "Should have exactly one start marker"
        );

        // SAFETY: We hold ENV_LOCK to prevent concurrent environment modifications
        unsafe {
            if let Some(home) = original_home {
                std::env::set_var("HOME", home);
            } else {
                std::env::remove_var("HOME");
            }
            if let Some(xdg) = original_xdg {
                std::env::set_var("XDG_CONFIG_HOME", xdg);
            } else {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
        }
    }
}
