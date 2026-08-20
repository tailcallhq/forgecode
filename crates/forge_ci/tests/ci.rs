use forge_ci::workflows as workflow;

const GENERATED_WORKFLOWS: [&str; 7] = [
    "autofix.yml",
    "bounty.yml",
    "ci.yml",
    "labels.yml",
    "release-drafter.yml",
    "release.yml",
    "stale.yml",
];

fn generated_workflow_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".github/workflows")
        .join(name)
}

#[test]
fn generated_workflows_are_parseable_and_identify_forge_ci_generator() {
    workflow::generate_autofix_workflow();
    workflow::generate_bounty_workflow();
    workflow::generate_ci_workflow();
    workflow::generate_labels_workflow();
    workflow::generate_release_drafter_workflow();
    workflow::release_publish();
    workflow::generate_stale_workflow();

    for name in GENERATED_WORKFLOWS {
        let generated = std::fs::read_to_string(generated_workflow_path(name))
            .expect("generated workflow should exist");
        let parsed = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&generated);

        assert!(parsed.is_ok(), "{name} must remain valid YAML");
        assert!(
            generated.contains("forge_ci"),
            "{name} must identify forge_ci as its generator"
        );
        assert!(
            !generated.contains("gh-workflow"),
            "{name} must not identify gh-workflow"
        );
    }

    let release = std::fs::read_to_string(generated_workflow_path("release.yml"))
        .expect("generated release workflow");
    assert!(release.contains("attest_release_assets:"));
    assert!(release.contains("needs: build_release"));

    let bounty = std::fs::read_to_string(generated_workflow_path("bounty.yml"))
        .expect("generated bounty workflow");
    assert!(bounty.contains(
        "if: github.event_name == 'pull_request' || github.event_name == 'pull_request_target'",
    ));
}

#[test]
fn generate() {
    workflow::generate_ci_workflow();
}

#[test]
fn test_release_drafter() {
    let expected = std::fs::read_to_string(generated_workflow_path("release-drafter.yml"))
        .expect("release drafter workflow baseline");
    workflow::generate_release_drafter_workflow();

    let actual = std::fs::read_to_string(generated_workflow_path("release-drafter.yml"))
        .expect("release drafter workflow output");
    assert!(
        !actual.contains("Auto Labeler"),
        "pull_request_target must not execute label writes"
    );
    assert!(actual.contains("contents: write"));
    assert!(actual.contains("pull-requests: read"));
    assert!(
        actual.contains("release-drafter/release-drafter@5a60cd8ddda6dc14fce77159675b8fd2cdca4007")
    );

    let expected = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&expected).unwrap();
    let actual = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&actual).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_release_workflow() {
    let expected = std::fs::read_to_string(generated_workflow_path("release.yml"))
        .expect("release workflow baseline");
    workflow::release_publish();

    let generated = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.github/workflows/release.yml"),
    )
    .expect("generated release workflow");
    assert!(!generated.contains("npm_release"));
    assert!(!generated.contains("homebrew_release"));
    assert!(generated.contains("Generate SHA-256 checksum"));
    assert!(generated.contains("shell: bash"));
    assert!(generated.contains("target: x86_64-unknown-linux-gnu"));
    assert!(generated.contains("target: x86_64-pc-windows-msvc"));
    assert!(generated.contains("matrix.binary_name }}.sha256"));
    assert!(generated.contains("attest_release_assets:"));
    assert!(generated.contains("needs: build_release"));
    assert!(generated.contains("attestations: write"));
    assert!(generated.contains("id-token: write"));
    assert!(generated.contains("gh release download"));
    assert!(generated.contains("--repo \"${{ github.repository }}\""));
    assert!(generated.contains("--pattern \"forge-*\""));
    assert!(generated.contains("--pattern \"helioslite-*\""));
    assert!(generated.contains("helioslite_name: helioslite-x86_64-unknown-linux-musl"));
    assert!(generated.contains("helioslite_name: helioslite-x86_64-pc-windows-msvc.exe"));
    assert!(generated.contains("Generate helioslite SHA-256 checksum"));
    assert!(generated.contains("Upload helioslite to Release"));
    assert!(generated.contains("Upload helioslite checksum to Release"));
    assert!(
        !generated.contains(": \n"),
        "release workflow must not contain trailing whitespace"
    );
    assert!(
        !generated.contains("\\\\\n"),
        "shell continuations must have exactly one trailing backslash"
    );
    assert!(generated.contains("actions/attest-build-provenance@"));

    let expected = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&expected).unwrap();
    let actual = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&generated).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_labels_workflow() {
    let expected = std::fs::read_to_string(generated_workflow_path("labels.yml"))
        .expect("labels workflow baseline");
    workflow::generate_labels_workflow();
    let actual = std::fs::read_to_string(generated_workflow_path("labels.yml"))
        .expect("labels workflow output");

    let expected = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&expected).unwrap();
    let actual = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&actual).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_stale_workflow() {
    let expected = std::fs::read_to_string(generated_workflow_path("stale.yml"))
        .expect("stale workflow baseline");
    workflow::generate_stale_workflow();
    let actual = std::fs::read_to_string(generated_workflow_path("stale.yml"))
        .expect("stale workflow output");

    assert!(actual.contains("cron: 0 * * * *"));
    assert!(actual.contains("issues: write"));
    assert!(actual.contains("pull-requests: write"));
    assert!(actual.contains("actions/stale@1e223db275d687790206a7acac4d1a11bd6fe629"));

    let expected = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&expected).unwrap();
    let actual = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&actual).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_autofix_workflow() {
    let expected = std::fs::read_to_string(generated_workflow_path("autofix.yml"))
        .expect("autofix workflow baseline");
    workflow::generate_autofix_workflow();
    let actual = std::fs::read_to_string(generated_workflow_path("autofix.yml"))
        .expect("autofix workflow output");

    assert!(actual.contains("cancel-in-progress: false"));
    assert!(actual.contains("contents: read"));
    assert!(actual.contains("actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803"));

    let expected = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&expected).unwrap();
    let actual = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&actual).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_bounty_workflow() {
    let expected = std::fs::read_to_string(generated_workflow_path("bounty.yml"))
        .expect("bounty workflow baseline");
    workflow::generate_bounty_workflow();

    let actual = std::fs::read_to_string(generated_workflow_path("bounty.yml"))
        .expect("bounty workflow output");
    assert!(actual.contains(
        "if: github.event_name == 'pull_request' || github.event_name == 'pull_request_target'",
    ));
    assert!(actual.contains("issues: write"));
    assert!(actual.contains("pull-requests: write"));
    assert!(actual.contains("sync-all-issues.ts"));

    let expected = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&expected).unwrap();
    let actual = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&actual).unwrap();
    assert_eq!(actual, expected);
}
