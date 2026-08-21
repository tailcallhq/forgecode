use serde::Serialize;
use serde_json::Value;

/// Matrix entry for build targets
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct MatrixEntry {
    pub os: &'static str,
    pub target: &'static str,
    pub binary_name: &'static str,
    pub binary_path: &'static str,
    pub cross: &'static str,
}

#[derive(Clone)]
pub struct ReleaseMatrix(Vec<MatrixEntry>);

impl Default for ReleaseMatrix {
    /// Returns a vector of all build matrix entries
    fn default() -> Self {
        ReleaseMatrix(vec![
            MatrixEntry {
                os: "ubuntu-latest",
                target: "x86_64-unknown-linux-musl",
                binary_name: "forge-x86_64-unknown-linux-musl",
                binary_path: "target/x86_64-unknown-linux-musl/release/forge",
                cross: "true",
            },
            MatrixEntry {
                os: "ubuntu-latest",
                target: "aarch64-unknown-linux-musl",
                binary_name: "forge-aarch64-unknown-linux-musl",
                binary_path: "target/aarch64-unknown-linux-musl/release/forge",
                cross: "true",
            },
            MatrixEntry {
                os: "ubuntu-latest",
                target: "x86_64-unknown-linux-gnu",
                binary_name: "forge-x86_64-unknown-linux-gnu",
                binary_path: "target/x86_64-unknown-linux-gnu/release/forge",
                cross: "false",
            },
            MatrixEntry {
                os: "ubuntu-latest",
                target: "aarch64-unknown-linux-gnu",
                binary_name: "forge-aarch64-unknown-linux-gnu",
                binary_path: "target/aarch64-unknown-linux-gnu/release/forge",
                cross: "true",
            },
            MatrixEntry {
                os: "macos-latest",
                target: "x86_64-apple-darwin",
                binary_name: "forge-x86_64-apple-darwin",
                binary_path: "target/x86_64-apple-darwin/release/forge",
                cross: "false",
            },
            MatrixEntry {
                os: "macos-latest",
                target: "aarch64-apple-darwin",
                binary_name: "forge-aarch64-apple-darwin",
                binary_path: "target/aarch64-apple-darwin/release/forge",
                cross: "false",
            },
            MatrixEntry {
                os: "windows-latest",
                target: "x86_64-pc-windows-msvc",
                binary_name: "forge-x86_64-pc-windows-msvc.exe",
                binary_path: "target/x86_64-pc-windows-msvc/release/forge.exe",
                cross: "false",
            },
            MatrixEntry {
                os: "windows-latest",
                target: "aarch64-pc-windows-msvc",
                binary_name: "forge-aarch64-pc-windows-msvc.exe",
                binary_path: "target/aarch64-pc-windows-msvc/release/forge.exe",
                cross: "false",
            },
            MatrixEntry {
                os: "ubuntu-latest",
                target: "aarch64-linux-android",
                binary_name: "forge-aarch64-linux-android",
                binary_path: "target/aarch64-linux-android/release/forge",
                cross: "true",
            },
        ])
    }
}

impl ReleaseMatrix {
    pub fn entries(&self) -> Vec<MatrixEntry> {
        self.0.clone()
    }
}

impl From<ReleaseMatrix> for Value {
    fn from(value: ReleaseMatrix) -> Self {
        serde_json::json!({
            "include": value.entries()
        })
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// Reusable fixture returning the default release matrix.
    fn fixture() -> ReleaseMatrix {
        ReleaseMatrix::default()
    }

    #[test]
    fn test_default_matrix_entry_count() {
        let actual = fixture().entries().len();

        let expected = 9;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_first_entry_is_linux_musl_cross_build() {
        let fixture = fixture();

        let actual = fixture
            .entries()
            .first()
            .expect("Matrix should not be empty")
            .clone();

        let expected = MatrixEntry {
            os: "ubuntu-latest",
            target: "x86_64-unknown-linux-musl",
            binary_name: "forge-x86_64-unknown-linux-musl",
            binary_path: "target/x86_64-unknown-linux-musl/release/forge",
            cross: "true",
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_targets_are_unique() {
        let fixture = fixture();

        let actual = {
            let mut targets: Vec<&str> = fixture.entries().iter().map(|e| e.target).collect();
            targets.sort_unstable();
            targets.dedup();
            targets.len()
        };

        let expected = fixture.entries().len();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_binary_name_matches_target_naming_convention() {
        let fixture = fixture();

        let actual: Vec<&str> = fixture
            .entries()
            .iter()
            .filter(|e| !e.binary_name.starts_with(&format!("forge-{}", e.target)))
            .map(|e| e.target)
            .collect();

        let expected: Vec<&str> = vec![];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_windows_entries_use_exe_extension() {
        let fixture = fixture();

        let actual: Vec<(&str, bool, bool)> = fixture
            .entries()
            .iter()
            .filter(|e| e.target.contains("windows"))
            .map(|e| {
                (
                    e.target,
                    e.binary_name.ends_with(".exe"),
                    e.binary_path.ends_with("forge.exe"),
                )
            })
            .collect();

        let expected = vec![
            ("x86_64-pc-windows-msvc", true, true),
            ("aarch64-pc-windows-msvc", true, true),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_binary_path_is_release_profile_for_target() {
        let fixture = fixture();

        let actual: Vec<&str> = fixture
            .entries()
            .iter()
            .filter(|e| {
                !e.binary_path
                    .starts_with(&format!("target/{}/release/", e.target))
            })
            .map(|e| e.target)
            .collect();

        let expected: Vec<&str> = vec![];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cross_flag_is_boolean_string() {
        let fixture = fixture();

        let actual: Vec<&str> = fixture
            .entries()
            .iter()
            .filter(|e| e.cross != "true" && e.cross != "false")
            .map(|e| e.target)
            .collect();

        let expected: Vec<&str> = vec![];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cross_compiled_targets() {
        let fixture = fixture();

        let actual: Vec<&str> = fixture
            .entries()
            .iter()
            .filter(|e| e.cross == "true")
            .map(|e| e.target)
            .collect();

        let expected = vec![
            "x86_64-unknown-linux-musl",
            "aarch64-unknown-linux-musl",
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-android",
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_runner_os_matches_target_platform() {
        let fixture = fixture();

        let actual: Vec<(&str, &str)> = fixture
            .entries()
            .iter()
            .map(|e| (e.target, e.os))
            .filter(|(target, os)| {
                let expected_os = if target.contains("apple") {
                    "macos-latest"
                } else if target.contains("windows") {
                    "windows-latest"
                } else {
                    "ubuntu-latest"
                };
                *os != expected_os
            })
            .collect();

        let expected: Vec<(&str, &str)> = vec![];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_into_value_wraps_entries_under_include_key() {
        let fixture = fixture();

        let actual = Value::from(fixture.clone());

        let expected = serde_json::json!({ "include": fixture.entries() });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_into_value_include_is_array_of_full_entries() {
        let fixture = fixture();

        let actual = Value::from(fixture)["include"][8].clone();

        let expected = serde_json::json!({
            "os": "ubuntu-latest",
            "target": "aarch64-linux-android",
            "binary_name": "forge-aarch64-linux-android",
            "binary_path": "target/aarch64-linux-android/release/forge",
            "cross": "true",
        });
        assert_eq!(actual, expected);
    }
}
