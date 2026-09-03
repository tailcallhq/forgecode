use derive_setters::Setters;

use crate::release_matrix::ReleaseMatrix;
use crate::steps::setup_protoc;
use crate::workflow_model::{Job, Level, Permissions, Step};

#[derive(Clone, Default, Setters)]
#[setters(strip_option, into)]
pub struct ReleaseBuilderJob {
    // Required to burn into the binary
    pub version: String,

    // When provide the generated release will be uploaded
    pub release_id: Option<String>,
}

impl ReleaseBuilderJob {
    pub fn new(version: impl AsRef<str>) -> Self {
        Self { version: version.as_ref().to_string(), release_id: None }
    }

    pub fn into_job(self) -> Job {
        self.into()
    }
}

impl From<ReleaseBuilderJob> for Job {
    fn from(value: ReleaseBuilderJob) -> Job {
        let permissions = if value.release_id.is_some() {
            Permissions::default().contents(Level::Write)
        } else {
            Permissions::default().contents(Level::Read)
        };

        let matrix: serde_json::Value = ReleaseMatrix::default().into();
        let mut job = Job::new("build-release")
            .strategy(serde_json::json!({"matrix": matrix}))
            .runs_on("${{ matrix.os }}")
            .permissions(permissions)
            .add_step(Step::new("Checkout Code").uses("actions", "checkout", "d23441a48e516b6c34aea4fa41551a30e30af803"))
            // Install protobuf compiler for non-cross builds
            // Cross builds install protoc via Cross.toml pre-build commands
            .add_step(
                setup_protoc().if_condition("${{ matrix.cross == 'false' }}"),
            )
            // Install Rust with cross-compilation target
            .add_step(
                Step::new("Setup Cross Toolchain")
                    .uses("taiki-e", "setup-cross-toolchain-action", "12b7ad4acfa95a1476779d6c06699b96ec1691f8")
                    .input("target", "${{ matrix.target }}")
                    .if_condition("${{ matrix.cross == 'false' }}"),
            )
            // Explicitly add the target to ensure it's available
            .add_step(
                Step::new("Add Rust target")
                    .run("rustup target add ${{ matrix.target }}")
                    .if_condition("${{ matrix.cross == 'false' }}"),
            )
            // Build add link flags
            .add_step(
                Step::new("Set Rust Flags")
                    .run(r#"echo "RUSTFLAGS=-C target-feature=+crt-static" >> "$GITHUB_ENV""#)
                    .if_condition(
                        "!(contains(matrix.target, '-unknown-linux-') || contains(matrix.target, '-android'))",
                    ),
            )
            // Build release binary
            // Note: protoc is installed via:
            // - arduino/setup-protoc action for non-cross builds
            // - Cross.toml pre-build commands for cross builds (apt-get install protobuf-compiler)
            .add_step(
                Step::new("Build Binary")
                    .uses("ClementTsang", "cargo-action", "2438cc5f3ba4e971289fffca2a00dedea6911f14")
                    .input("command", "build --release")
                    .input("args", "--target ${{ matrix.target }}")
                    .input("use-cross", "${{ matrix.cross }}")
                    .input("cross-version", "0.2.5")
                    .env("POSTHOG_API_SECRET", "${{secrets.POSTHOG_API_SECRET}}")
                    .env("APP_VERSION", value.version.to_string()),
            )
            .add_step(
                Step::new("Build forge_dbd Binary")
                    .uses("ClementTsang", "cargo-action", "2438cc5f3ba4e971289fffca2a00dedea6911f14")
                    .input("command", "build --release")
                    .input("args", "--target ${{ matrix.target }} -p forge_dbd")
                    .input("use-cross", "${{ matrix.cross }}")
                    .input("cross-version", "0.2.5")
                    .env("APP_VERSION", value.version.to_string()),
            )
            // Build the Windows self-update helper binary (see
            // forge_main/src/bin/helioslite_helper.rs). Without this binary
            // the running forge.exe falls back to the legacy PS1 scaffolder
            // in forge_main::update::windows_update_command, which is the
            // pre-refactor path that was the source of the
            // ": not recognized" + IO error bugs. Conditional on Windows
            // because the helper is winapi-only.
            .add_step(
                Step::new("Build helioslite_helper Binary")
                    .uses("ClementTsang", "cargo-action", "2438cc5f3ba4e971289fffca2a00dedea6911f14")
                    .input("command", "build --release")
                    .input("args", "--target ${{ matrix.target }} -p forge_main --bin helioslite_helper")
                    .input("use-cross", "${{ matrix.cross }}")
                    .input("cross-version", "0.2.5")
                    .env("APP_VERSION", value.version.to_string())
                    .if_condition("contains(matrix.target, 'windows')"),
            );

        if let Some(release_id) = value.release_id {
            job = job
                // Rename binary to the forge asset name and mirror it under
                // the helioslite asset name so both binary identities can
                // self-update from this release (see forge_main::update).
                .add_step(
                    Step::new("Copy Binary")
                        .run("cp ${{ matrix.binary_path }} ${{ matrix.binary_name }}\ncp ${{ matrix.binary_path }} ${{ matrix.helioslite_name }}"),
                )
                .add_step(
                    Step::new("Generate SHA-256 checksum")
                        .run(r#"if command -v sha256sum >/dev/null 2>&1; then sha256sum "${{ matrix.binary_name }}" > "${{ matrix.binary_name }}.sha256"; else shasum -a 256 "${{ matrix.binary_name }}" > "${{ matrix.binary_name }}.sha256"; fi"#)
                        .shell("bash"),
                )
                .add_step(
                    Step::new("Generate helioslite SHA-256 checksum")
                        .run(r#"if command -v sha256sum >/dev/null 2>&1; then sha256sum "${{ matrix.helioslite_name }}" > "${{ matrix.helioslite_name }}.sha256"; else shasum -a 256 "${{ matrix.helioslite_name }}" > "${{ matrix.helioslite_name }}.sha256"; fi"#)
                        .shell("bash"),
                )
                // Upload to the generated github release id
                .add_step(
                    Step::new("Upload to Release")
                        .uses(
                            "xresloader",
                            "upload-to-github-release",
                            "7c5757a90c0bcf0c0e1741da8f2abd7b85e675d0",
                        )
                        .input("release_id", release_id.clone())
                        .input("file", "${{ matrix.binary_name }}")
                        .input("overwrite", "true"),
                )
                .add_step(
                    Step::new("Upload checksum to Release")
                        .uses(
                            "xresloader",
                            "upload-to-github-release",
                            "7c5757a90c0bcf0c0e1741da8f2abd7b85e675d0",
                        )
                        .input("release_id", release_id.clone())
                        .input("file", "${{ matrix.binary_name }}.sha256")
                        .input("overwrite", "true"),
                )
                .add_step(
                    Step::new("Upload helioslite to Release")
                        .uses(
                            "xresloader",
                            "upload-to-github-release",
                            "7c5757a90c0bcf0c0e1741da8f2abd7b85e675d0",
                        )
                        .input("release_id", release_id.clone())
                        .input("file", "${{ matrix.helioslite_name }}")
                        .input("overwrite", "true"),
                )
                .add_step(
                    Step::new("Upload helioslite checksum to Release")
                        .uses(
                            "xresloader",
                            "upload-to-github-release",
                            "7c5757a90c0bcf0c0e1741da8f2abd7b85e675d0",
                        )
                        .input("release_id", release_id.clone())
                        .input("file", "${{ matrix.helioslite_name }}.sha256")
                        .input("overwrite", "true"),
                )
                .add_step(
                    Step::new("Copy forge_dbd Binary")
                        .run("if [[ \"${{ matrix.target }}\" == *windows* ]]; then cp \"target/${{ matrix.target }}/release/forge_dbd.exe\" \"forge_dbd-${{ matrix.target }}.exe\"; else cp \"target/${{ matrix.target }}/release/forge_dbd\" \"forge_dbd-${{ matrix.target }}\"; fi")
                        .shell("bash"),
                )
                .add_step(
                    Step::new("Generate forge_dbd SHA-256")
                        .run("if [[ \"${{ matrix.target }}\" == *windows* ]]; then if command -v sha256sum >/dev/null 2>&1; then sha256sum \"forge_dbd-${{ matrix.target }}.exe\" > \"forge_dbd-${{ matrix.target }}.exe.sha256\"; else shasum -a 256 \"forge_dbd-${{ matrix.target }}.exe\" > \"forge_dbd-${{ matrix.target }}.exe.sha256\"; fi; else if command -v sha256sum >/dev/null 2>&1; then sha256sum \"forge_dbd-${{ matrix.target }}\" > \"forge_dbd-${{ matrix.target }}.sha256\"; else shasum -a 256 \"forge_dbd-${{ matrix.target }}\" > \"forge_dbd-${{ matrix.target }}.sha256\"; fi; fi")
                        .shell("bash"),
                )
                .add_step(
                    Step::new("Upload forge_dbd to Release (unix)")
                        .uses(
                            "xresloader",
                            "upload-to-github-release",
                            "7c5757a90c0bcf0c0e1741da8f2abd7b85e675d0",
                        )
                        .input("release_id", release_id.clone())
                        .input("file", "forge_dbd-${{ matrix.target }}")
                        .input("overwrite", "true")
                        .if_condition("!contains(matrix.target, 'windows')"),
                )
                .add_step(
                    Step::new("Upload forge_dbd checksum to Release (unix)")
                        .uses(
                            "xresloader",
                            "upload-to-github-release",
                            "7c5757a90c0bcf0c0e1741da8f2abd7b85e675d0",
                        )
                        .input("release_id", release_id.clone())
                        .input("file", "forge_dbd-${{ matrix.target }}.sha256")
                        .input("overwrite", "true")
                        .if_condition("!contains(matrix.target, 'windows')"),
                )
                .add_step(
                    Step::new("Upload forge_dbd to Release (windows)")
                        .uses(
                            "xresloader",
                            "upload-to-github-release",
                            "7c5757a90c0bcf0c0e1741da8f2abd7b85e675d0",
                        )
                        .input("release_id", release_id.clone())
                        .input("file", "forge_dbd-${{ matrix.target }}.exe")
                        .input("overwrite", "true")
                        .if_condition("contains(matrix.target, 'windows')"),
                )
                .add_step(
                    Step::new("Upload forge_dbd checksum to Release (windows)")
                        .uses(
                            "xresloader",
                            "upload-to-github-release",
                            "7c5757a90c0bcf0c0e1741da8f2abd7b85e675d0",
                        )
                        .input("release_id", release_id.clone())
                        .input("file", "forge_dbd-${{ matrix.target }}.exe.sha256")
                        .input("overwrite", "true")
                        .if_condition("contains(matrix.target, 'windows')"),
                )
                // Windows self-update helper: copy from cargo target dir to
                // the asset name, generate a sidecar SHA-256, upload both.
                // Windows-only because the helper is a winapi-only binary
                // (it uses OpenProcess/WaitForSingleObject/MoveFileExW/
                // CreateProcessW). The runner locates the helper next to
                // the running exe and spawns it with `helioslite_helper.exe
                // download <repo> <asset> wait <pid> swap <from> <to>`.
                .add_step(
                    Step::new("Copy helioslite_helper Binary")
                        .run("cp \"${{ matrix.helper_path }}\" \"${{ matrix.helper_name }}\"")
                        .shell("bash")
                        .if_condition("contains(matrix.target, 'windows')"),
                )
                .add_step(
                    Step::new("Generate helioslite_helper SHA-256 checksum")
                        .run(r#"if command -v sha256sum >/dev/null 2>&1; then sha256sum "${{ matrix.helper_name }}" > "${{ matrix.helper_name }}.sha256"; else shasum -a 256 "${{ matrix.helper_name }}" > "${{ matrix.helper_name }}.sha256"; fi"#)
                        .shell("bash")
                        .if_condition("contains(matrix.target, 'windows')"),
                )
                .add_step(
                    Step::new("Upload helioslite_helper to Release (windows)")
                        .uses(
                            "xresloader",
                            "upload-to-github-release",
                            "7c5757a90c0bcf0c0e1741da8f2abd7b85e675d0",
                        )
                        .input("release_id", release_id.clone())
                        .input("file", "${{ matrix.helper_name }}")
                        .input("overwrite", "true")
                        .if_condition("contains(matrix.target, 'windows')"),
                )
                .add_step(
                    Step::new("Upload helioslite_helper checksum to Release (windows)")
                        .uses(
                            "xresloader",
                            "upload-to-github-release",
                            "7c5757a90c0bcf0c0e1741da8f2abd7b85e675d0",
                        )
                        .input("release_id", release_id)
                        .input("file", "${{ matrix.helper_name }}.sha256")
                        .input("overwrite", "true")
                        .if_condition("contains(matrix.target, 'windows')"),
                );
        }

        job
    }
}
