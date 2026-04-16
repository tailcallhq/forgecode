//! VS Code terminal detection and automatic extension installation

use std::process::Command;

/// Checks if running in VS Code terminal
///
/// Detects VS Code by checking for environment variables that are set
/// when running in the VS Code integrated terminal.
pub fn is_vscode_terminal() -> bool {
    std::env::var("TERM_PROGRAM")
        .map(|val| val == "vscode")
        .unwrap_or(false)
        || std::env::var("VSCODE_PID").is_ok()
        || std::env::var("VSCODE_GIT_ASKPASS_NODE").is_ok()
        || std::env::var("VSCODE_GIT_IPC_HANDLE").is_ok()
}

/// Checks if the Forge VS Code extension is installed
///
/// Checks VS Code's extension list to see if ForgeCode.forge-vscode is
/// installed.
pub fn is_extension_installed() -> bool {
    // Try to list installed extensions
    if let Ok(output) = Command::new("code").arg("--list-extensions").output()
        && output.status.success()
        && let Ok(extensions) = String::from_utf8(output.stdout)
    {
        return extensions
            .lines()
            .any(|line| line.trim() == "ForgeCode.forge-vscode");
    }
    false
}

/// Attempts to install the Forge VS Code extension silently
///
/// Returns Ok(true) if installation was successful, Ok(false) if it failed,
/// or Err if the command couldn't be executed.
pub fn install_extension() -> Result<bool, std::io::Error> {
    let output = Command::new("code")
        .arg("--install-extension")
        .arg("ForgeCode.forge-vscode")
        .arg("--force")
        .output()?;

    Ok(output.status.success())
}

/// Returns true if we should install the extension
///
/// This will return true only when:
/// - Running in VS Code terminal
/// - Extension is not installed
pub fn should_install_extension() -> bool {
    is_vscode_terminal() && !is_extension_installed()
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::OsString;
    use std::sync::{LazyLock, Mutex, MutexGuard};

    use super::*;

    static ENV_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct EnvGuard {
        key: &'static str,
        original_value: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|error| error.into_inner());
            let original_value = env::var_os(key);
            unsafe {
                env::set_var(key, value);
            }
            Self { key, original_value, _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original_value {
                Some(value) => unsafe {
                    env::set_var(self.key, value);
                },
                None => unsafe {
                    env::remove_var(self.key);
                },
            }
        }
    }

    fn with_env_var<F>(key: &'static str, value: &str, test: F)
    where
        F: FnOnce(),
    {
        let _guard = EnvGuard::set(key, value);
        test();
    }

    #[test]
    fn test_is_vscode_terminal_with_term_program() {
        with_env_var("TERM_PROGRAM", "vscode", || {
            assert!(is_vscode_terminal());
        });
    }

    #[test]
    fn test_is_vscode_terminal_with_vscode_pid() {
        with_env_var("VSCODE_PID", "12345", || {
            assert!(is_vscode_terminal());
        });
    }

    #[test]
    fn test_is_vscode_terminal_with_git_askpass() {
        with_env_var("VSCODE_GIT_ASKPASS_NODE", "/path/to/node", || {
            assert!(is_vscode_terminal());
        });
    }

    #[test]
    fn test_is_vscode_terminal_with_git_ipc() {
        with_env_var("VSCODE_GIT_IPC_HANDLE", "handle", || {
            assert!(is_vscode_terminal());
        });
    }

    #[test]
    fn test_should_install_when_in_vscode() {
        with_env_var("TERM_PROGRAM", "vscode", || {
            // We can't reliably test the actual installation check since it depends
            // on the actual VS Code installation, but we can verify the logic
            // when in VS Code terminal
            assert!(is_vscode_terminal());
        });
    }
}
