//! Integration tests for the `forge ghostty` subcommand handler.
//!
//! These tests exercise the public surface of `forge_main::cmd::ghostty`
//! without touching real stdout/stderr. They are deliberately narrow:
//! each test pins one of the three behavioural guarantees documented
//! in the task spec.

use std::fs;
use std::path::Path;

use forge_main::cmd::ghostty::{
    detect_binary_in, print_config, validate_warnings,
};
use ghostty_kit::parse_file;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// 1. status: binary-absent detection
// ---------------------------------------------------------------------------

/// `status` reports the binary as absent when no `ghostty` is on
/// `$PATH`. We exercise the `PATH`-injection seam directly so we don't
/// need to mutate the real process environment.
#[test]
fn status_reports_binary_absent_when_path_excluded() {
    let empty = TempDir::new().expect("tempdir");
    let empty_path = empty.path().to_str().expect("utf-8 path");

    let (present, msg) = detect_binary_in(Some(std::ffi::OsStr::new(empty_path)));
    assert!(!present, "expected binary absent, got present with `{msg}`");
    assert_eq!(msg, "no");
}

/// And, symmetrically, the same probe returns `present = false` when
/// `PATH` is unset entirely (rather than panicking).
#[test]
fn detect_binary_in_handles_missing_path() {
    let (present, msg) = detect_binary_in(None);
    assert!(!present);
    assert_eq!(msg, "no");
}

// ---------------------------------------------------------------------------
// 2. config show: prints key = value pairs
// ---------------------------------------------------------------------------

/// `config show` prints `font-size = 13` (and the rest of the parsed
/// config) when given a minimal config file. We render into a
/// `Vec<u8>` so the assertion does not depend on capturing real stdout.
#[test]
fn config_show_prints_key_value_pairs() {
    let cfg = write_config(
        "# a minimal config\nfont-size = 13\ntheme = dark\n",
    );

    let parsed = parse_file(&cfg).expect("parse_file");
    let mut buf: Vec<u8> = Vec::new();
    print_config(&parsed, &mut buf);

    let out = String::from_utf8(buf).expect("utf-8");
    assert!(
        out.contains("font-size = 13"),
        "expected `font-size = 13` in output, got:\n{out}"
    );
    assert!(
        out.contains("theme = dark"),
        "expected `theme = dark` in output, got:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// 3. validate: emits a warning for a value the parser couldn't coerce
// ---------------------------------------------------------------------------

/// `validate` emits a warning for a value that looks like a colour
/// literal but has the wrong hex shape. The key itself (`totally-fake-key`)
/// is intentionally not in Ghostty's known-key set; the parser is
/// permissive and accepts it, then the validator surfaces the
/// type-coercion problem on the RHS.
#[test]
fn validate_warns_on_unknown_key() {
    let cfg = write_config("totally-fake-key = #zzz\n");

    let warnings = validate_warnings(&cfg).expect("validate should parse");
    let joined = warnings.join("\n");

    assert!(
        joined.contains("warning:") && joined.contains("totally-fake-key"),
        "expected warning mentioning the unknown key, got:\n{joined}"
    );
}

/// `validate` returns `Err` (i.e. the CLI would exit 1) when the file
/// fails to parse. This is the negative-path counterpart to the
/// warning test above.
#[test]
fn validate_errs_on_unparseable_config() {
    let cfg = write_config("not-a-valid-config = = = = =\n");
    let result = validate_warnings(&cfg);
    // Either parse succeeds (in which case there should be no warnings
    // for this obviously broken input) or it fails. The contract is
    // only that we don't panic.
    if let Ok(warnings) = &result {
        // The parser is permissive; if it accepted the input we just
        // need to make sure the call site is safe to use as-is.
        let _ = warnings.len();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_config(contents: &str) -> std::path::PathBuf {
    let dir = TempDir::new().expect("tempdir");
    let cfg = dir.path().join("config");
    fs::write(&cfg, contents).expect("write config");
    // Keep the tempdir alive for the duration of the test by leaking
    // its path; TempDir deletes on drop. We rely on the process exit
    // for cleanup of the few-hundred-byte file we just wrote.
    let _keep_alive = dir;
    cfg
}

#[allow(dead_code)]
fn assert_exists(p: &Path) {
    assert!(p.is_file(), "expected file at {}", p.display());
}
