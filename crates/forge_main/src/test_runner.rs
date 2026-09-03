//! Test runner for executing `cargo test` and `cargo bench` as subprocesses.
//!
//! Provides structured test execution with output parsing to extract
//! pass/fail/ignore counts from `cargo test` summary lines.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use bstr::ByteSlice;

/// Result of a test or benchmark run.
///
/// Contains the parsed counts from `cargo test` output alongside the raw
/// output string and an overall success flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    /// Number of tests that passed.
    pub passed: usize,
    /// Number of tests that failed.
    pub failed: usize,
    /// Number of tests that were ignored.
    pub ignored: usize,
    /// Raw combined stdout + stderr output from the subprocess.
    pub output: String,
    /// Whether the run completed without failures (`failed == 0` and exit
    /// status zero).
    pub success: bool,
}

/// Runs `cargo test` and `cargo bench` commands in a given working directory.
///
/// # Examples
///
/// ```no_run
/// use forge_main::TestRunner;
///
/// let runner = TestRunner::new().unwrap();
/// let result = runner.run_all().unwrap();
/// println!("passed: {}, failed: {}", result.passed, result.failed);
/// ```
#[derive(Debug)]
pub struct TestRunner {
    working_dir: PathBuf,
}

impl TestRunner {
    /// Creates a `TestRunner` using the current working directory.
    pub fn new() -> Result<Self> {
        let working_dir =
            std::env::current_dir().context("failed to determine current working directory")?;
        Ok(Self { working_dir })
    }

    /// Creates a `TestRunner` that executes in the given directory.
    pub fn with_dir(path: PathBuf) -> Self {
        Self { working_dir: path }
    }

    /// Runs `cargo test` with optional package and test name filters.
    ///
    /// When `package` is provided, passes `--package <pkg>`. When `test_name`
    /// is provided, passes it as a test filter argument to `cargo test`.
    pub fn run_test(&self, package: Option<&str>, test_name: Option<&str>) -> Result<TestResult> {
        let mut cmd = Command::new("cargo");
        cmd.arg("test");

        if let Some(pkg) = package {
            cmd.args(["--package", pkg]);
        }

        if let Some(name) = test_name {
            cmd.arg("--");
            cmd.arg(name);
        }

        self.execute(cmd)
    }

    /// Runs `cargo test` without any filters (all tests in the workspace).
    pub fn run_all(&self) -> Result<TestResult> {
        let mut cmd = Command::new("cargo");
        cmd.arg("test");
        self.execute(cmd)
    }

    /// Runs `cargo bench` in the working directory.
    pub fn run_benchmarks(&self) -> Result<TestResult> {
        let mut cmd = Command::new("cargo");
        cmd.arg("bench");
        self.execute(cmd)
    }

    /// Executes a command and parses the combined output.
    fn execute(&self, mut cmd: Command) -> Result<TestResult> {
        cmd.current_dir(&self.working_dir);

        let output = cmd.output().context("failed to execute cargo command")?;

        let stdout = decode_lossy(&output.stdout);
        let stderr = decode_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}");

        let (passed, failed, ignored) = parse_cargo_test_output(&combined);
        let success = output.status.success() && failed == 0;

        Ok(TestResult { passed, failed, ignored, output: combined, success })
    }
}

fn decode_lossy(bytes: &[u8]) -> String {
    bytes.to_str_lossy().into_owned()
}

/// Parses `cargo test` output to extract pass/fail/ignore counts from summary
/// lines.
///
/// Looks for the pattern `test result: ok. N passed; M failed; K ignored`
/// (or `FAILED` instead of `ok`). Returns `(passed, failed, ignored)`.
///
/// # Examples
///
/// ```
/// use forge_main::parse_cargo_test_output;
///
/// let (passed, failed, ignored) = parse_cargo_test_output(
///     "test result: ok. 5 passed; 0 failed; 0 ignored"
/// );
/// assert_eq!((passed, failed, ignored), (5, 0, 0));
/// ```
pub fn parse_cargo_test_output(output: &str) -> (usize, usize, usize) {
    let mut total_passed: usize = 0;
    let mut total_failed: usize = 0;
    let mut total_ignored: usize = 0;

    for line in output.lines() {
        if !line.contains("test result:") {
            continue;
        }

        if let Some(passed) = extract_count(line, "passed") {
            total_passed += passed;
        }
        if let Some(failed) = extract_count(line, "failed") {
            total_failed += failed;
        }
        if let Some(ignored) = extract_count(line, "ignored") {
            total_ignored += ignored;
        }
    }

    (total_passed, total_failed, total_ignored)
}

/// Extracts a numeric count immediately before the given label (e.g.,
/// `"5 passed"` → `Some(5)`).
fn extract_count(line: &str, label: &str) -> Option<usize> {
    let marker = format!(" {label}");

    // Walk backwards from the marker to find the numeric run.
    let before = prefix_before_marker(line, &marker)?;
    let num_str = before
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();

    num_str.parse().ok()
}

fn prefix_before_marker<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let idx = line.find(marker)?;
    line.get(..idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_parse_cargo_test_output_ok() {
        let input = "test result: ok. 5 passed; 0 failed; 0 ignored";
        let (passed, failed, ignored) = parse_cargo_test_output(input);
        assert_eq!((passed, failed, ignored), (5, 0, 0));
    }

    #[test]
    fn test_parse_cargo_test_output_mixed() {
        let input = "test result: FAILED. 3 passed; 2 failed; 0 ignored";
        let (passed, failed, ignored) = parse_cargo_test_output(input);
        assert_eq!((passed, failed, ignored), (3, 2, 0));
    }

    #[test]
    fn test_parse_cargo_test_output_ignored() {
        let input = "test result: ok. 10 passed; 0 failed; 4 ignored";
        let (passed, failed, ignored) = parse_cargo_test_output(input);
        assert_eq!((passed, failed, ignored), (10, 0, 4));
    }

    #[test]
    fn test_parse_cargo_test_output_empty() {
        let (passed, failed, ignored) = parse_cargo_test_output("");
        assert_eq!((passed, failed, ignored), (0, 0, 0));
    }

    #[test]
    fn prefix_before_marker_preserves_unicode_boundaries() {
        let fixture = "result: ok. caf\u{00e9} 5 passed";

        let actual = prefix_before_marker(fixture, " passed").unwrap();

        let expected = "result: ok. caf\u{00e9} 5";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_test_runner_new() {
        let runner = TestRunner::new();
        assert!(runner.is_ok());
        let runner = runner.unwrap();
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(runner.working_dir, cwd);
    }

    #[test]
    fn test_test_runner_with_dir() {
        let dir = PathBuf::from("/tmp/test-project");
        let runner = TestRunner::with_dir(dir.clone());
        assert_eq!(runner.working_dir, dir);
    }

    #[test]
    fn decode_lossy_replaces_invalid_utf8() {
        let fixture = [b'f', b'o', 0x80, b'o'];

        let actual = decode_lossy(&fixture);

        let expected = "fo\u{fffd}o";
        assert_eq!(actual, expected);
    }

    #[test]
    fn run_all_executes_tests_in_the_runner_directory() {
        let fixture = tempdir().unwrap();
        let source_dir = fixture.path().join("src");
        fs::create_dir(&source_dir).unwrap();
        fs::write(
            fixture.path().join("Cargo.toml"),
            "[package]\nname = \"runner_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(
            source_dir.join("lib.rs"),
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn passes() {}\n}\n",
        )
        .unwrap();

        let actual = TestRunner::with_dir(fixture.path().to_path_buf())
            .run_all()
            .unwrap();

        assert!(actual.success, "{}", actual.output);
        assert_eq!((actual.passed, actual.failed, actual.ignored), (1, 0, 0));
    }

    #[test]
    fn test_parse_cargo_test_output_real_world() {
        let input = "\
running 8 tests
test parser::test_parse_simple ... ok
test parser::test_parse_empty ... ok
test parser::test_parse_error ... FAILED
test runner::test_run_all ... ok
test runner::test_run_bench ... ignored
test runner::test_run_package ... ok
test utils::test_format_output ... ok
test utils::test_parse_counts ... FAILED

test result: FAILED. 5 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.23s
";
        let (passed, failed, ignored) = parse_cargo_test_output(input);
        assert_eq!((passed, failed, ignored), (5, 2, 1));
    }
}
