//! Tests for the sandbox crate.
//!
//! Most tests use the `Disabled` backend because we can't reliably create a
//! Landlock ruleset in a unit-test environment. The platform-specific
//! backends have their own integration tests in `tests/` (TODO).

#[cfg(test)]
mod sandbox_tests {
    use crate::backend::Backend;
    use crate::config::SandboxConfig;
    use crate::disabled::DisabledBackend;

    #[tokio::test]
    async fn disabled_backend_runs_command() {
        let backend = DisabledBackend::new();
        let config = SandboxConfig::builder()
            .command(if cfg!(windows) { "cmd" } else { "sh" })
            .args(if cfg!(windows) {
                vec!["/C".to_string(), "echo hello".to_string()]
            } else {
                vec!["-c".to_string(), "echo hello".to_string()]
            })
            .build();

        let output = backend.run(&config).await.expect("run");
        assert!(output.stdout.contains("hello"));
        assert!(!output.sandboxed);
        assert_eq!(output.exit_code, 0);
    }

    #[tokio::test]
    async fn disabled_backend_captures_exit_code() {
        let backend = DisabledBackend::new();
        let config = SandboxConfig::builder()
            .command(if cfg!(windows) { "cmd" } else { "sh" })
            .args(if cfg!(windows) {
                vec!["/C".to_string(), "exit 7".to_string()]
            } else {
                vec!["-c".to_string(), "exit 7".to_string()]
            })
            .build();

        let output = backend.run(&config).await.expect("run");
        assert_eq!(output.exit_code, 7);
        assert!(output.failed());
    }

    #[tokio::test]
    async fn disabled_backend_returns_io_error_for_missing_command() {
        let backend = DisabledBackend::new();
        let config = SandboxConfig::builder()
            .command("definitely-not-a-real-binary-xyz123")
            .build();

        let result = backend.run(&config).await;
        assert!(result.is_err());
    }

    #[test]
    fn sandbox_dispatch_returns_a_backend() {
        let sandbox = crate::backend::Sandbox::for_platform();
        // On every supported platform, for_platform() returns *something*.
        let _ = sandbox.name();
        // enforces_isolation may be false (passthrough) — we don't assert.
    }

    #[test]
    fn sandbox_config_rejects_nul_byte() {
        let cfg = SandboxConfig::builder()
            .command("echo\0malicious")
            .working_dir(std::env::temp_dir())
            .build();
        assert!(cfg.validate().is_err());
    }
}
