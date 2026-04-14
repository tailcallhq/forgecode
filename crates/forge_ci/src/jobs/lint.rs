//! Shared lint commands for CI workflows

/// Build a cargo command from parts
fn cargo_cmd(parts: &[&str]) -> String {
    parts.join(" ")
}

/// Base parts for fmt commands
fn fmt_base() -> Vec<&'static str> {
    vec!["cargo", "+nightly", "fmt", "--all"]
}

/// Available cargo clippy command profiles for CI.
#[derive(Clone, Copy)]
pub enum ClippyProfile {
    /// Run the default lint configuration and fail on warnings.
    DenyWarnings,
    /// Run UTF-8 and indexing safety lints for string and slice operations.
    StringSafety,
}

/// Base parts for clippy commands.
///
/// The `StringSafety` profile omits `--all-targets` so that test code is
/// excluded from the check.
fn clippy_base(profile: ClippyProfile) -> Vec<&'static str> {
    let mut parts = vec![
        "cargo",
        "+nightly",
        "clippy",
        "--all-features",
        "--workspace",
    ];
    if matches!(profile, ClippyProfile::DenyWarnings) {
        parts.push("--all-targets");
    }
    parts
}

/// Additional lint arguments for clippy commands.
fn clippy_lints(profile: ClippyProfile) -> Vec<&'static str> {
    match profile {
        ClippyProfile::DenyWarnings => vec!["-D", "warnings"],
        ClippyProfile::StringSafety => {
            vec![
                "-W",
                "clippy::string_slice",
                "-W",
                "clippy::indexing_slicing",
            ]
        }
    }
}

/// Build a cargo fmt command
pub fn fmt_cmd(fix: bool) -> String {
    let mut parts = fmt_base();
    if !fix {
        parts.push("--check");
    }
    cargo_cmd(&parts)
}

/// Build a cargo clippy command for a CI profile.
pub fn clippy_cmd(fix: bool, profile: ClippyProfile) -> String {
    let mut parts = clippy_base(profile);

    if fix {
        parts.extend(["--fix", "--allow-dirty"]);
    }

    parts.push("--");
    parts.extend(clippy_lints(profile));

    cargo_cmd(&parts)
}
