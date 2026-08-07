use forge_ci::workflows as workflow;

#[test]
fn generate() {
    workflow::generate_ci_workflow();
}

#[test]
fn generated_ci_preserves_blocking_pr_security_scans() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("generated ci workflow is readable");

    assert!(workflow.contains("dependency_review:"));
    assert!(workflow.contains("Dependency Review"));
    assert!(workflow
        .contains("actions/dependency-review-action@a1d282b36b6f3519aa1f3fc636f609c47dddb294"));
    assert!(workflow.contains("if: github.event_name == 'pull_request'"));
    assert!(workflow.contains("trivy:"));
    assert!(workflow.contains("Filesystem and Dependency Vulnerability Scan"));
    assert!(workflow.contains("aquasecurity/trivy-action@ed142fd0673e97e23eac54620cfb913e5ce36c25"));
    assert!(workflow.contains("persist-credentials: 'false'"));
    assert!(workflow.contains("scan-type: fs"));
    assert!(workflow.contains("scanners: vuln"));
    assert!(workflow.contains("exit-code: '1'"));
    assert!(workflow.ends_with('\n'));
    assert!(workflow.lines().all(|line| line.trim_end() == line));
}

#[test]
fn test_release_drafter() {
    workflow::generate_release_drafter_workflow();
}

#[test]
fn test_release_workflow() {
    workflow::release_publish();
}

#[test]
fn test_labels_workflow() {
    workflow::generate_labels_workflow();
}

#[test]
fn test_stale_workflow() {
    workflow::generate_stale_workflow();
}

#[test]
fn test_autofix_workflow() {
    workflow::generate_autofix_workflow();
}

#[test]
fn test_bounty_workflow() {
    workflow::generate_bounty_workflow();
}
