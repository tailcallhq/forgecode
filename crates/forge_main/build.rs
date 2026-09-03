fn clean_version(version: &str) -> String {
    // Remove 'v' prefix if present using strip_prefix
    version.strip_prefix('v').unwrap_or(version).to_string()
}

fn main() {
    // Priority order:
    // 1. APP_VERSION environment variable (for CI/CD builds)
    // 2. Package version from Cargo.toml for local/source builds

    let version = std::env::var("APP_VERSION")
        .map(|v| clean_version(&v))
        .unwrap_or_else(|_| clean_version(&std::env::var("CARGO_PKG_VERSION").unwrap()));

    // Make version available to the application
    println!("cargo:rustc-env=CARGO_PKG_VERSION={version}");

    // Ensure rebuild when environment changes
    println!("cargo:rerun-if-env-changed=APP_VERSION");
}
