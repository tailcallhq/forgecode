fn clean_version(version: &str) -> String {
    // Remove 'v' prefix if present using strip_prefix
    version.strip_prefix('v').unwrap_or(version).to_string()
}

fn get_git_version() -> Option<String> {
    use std::process::Command;
    let output = Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .ok()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(clean_version(&version))
    } else {
        None
    }
}

fn main() {
    // Priority order:
    // 1. APP_VERSION environment variable (for CI/CD builds)
    // 2. Version from git tags
    // 3. Fallback to dev version

    let version = std::env::var("APP_VERSION")
        .map(|v| clean_version(&v))
        .or_else(|_| get_git_version().ok_or(()))
        .unwrap_or_else(|_| "2.11.5".to_string());

    // Make version available to the application
    println!("cargo:rustc-env=CARGO_PKG_VERSION={version}");

    // Make version available to the application
    println!("cargo:rustc-env=CARGO_PKG_NAME=forge");

    // Ensure rebuild when environment changes
    println!("cargo:rerun-if-env-changed=APP_VERSION");
    println!("cargo:rerun-if-changed=.git/refs/heads");
    println!("cargo:rerun-if-changed=.git/refs/tags");
}
