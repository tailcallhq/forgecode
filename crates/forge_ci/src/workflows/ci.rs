use gh_workflow::generate::Generate;
use gh_workflow::*;

use crate::jobs::{self, ReleaseBuilderJob};
use crate::steps::setup_protoc;

/// Generate the main CI workflow
pub fn generate_ci_workflow() {
    // Create a basic build job for CI with coverage
    let build_job = Job::new("Build and Test")
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Checkout Code").uses("actions", "checkout", "v6"))
        .add_step(setup_protoc())
        .add_step(Step::toolchain().add_stable())
        .add_step(Step::new("Install cargo-llvm-cov").run("cargo install cargo-llvm-cov"))
        .add_step(
            Step::new("Generate coverage")
                .run("cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info"),
        );

    // Create a performance test job to ensure zsh rprompt stays fast
    let perf_test_job = Job::new("zsh-rprompt-performance")
        .name("Performance: zsh rprompt")
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Checkout Code").uses("actions", "checkout", "v6"))
        .add_step(setup_protoc())
        .add_step(Step::toolchain().add_stable())
        .add_step(
            Step::new("Run performance benchmark")
                .run("./scripts/benchmark.sh --threshold 60 zsh rprompt"),
        );

    let dependency_review_job = Job::new("Dependency Review")
        .cond(Expression::new("github.event_name == 'pull_request'"))
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Dependency Review").uses(
            "actions",
            "dependency-review-action",
            "a1d282b36b6f3519aa1f3fc636f609c47dddb294",
        ));

    let trivy_job = Job::new("Filesystem and Dependency Vulnerability Scan")
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(
            Step::new("Checkout Code")
                .uses(
                    "actions",
                    "checkout",
                    "d23441a48e516b6c34aea4fa41551a30e30af803",
                )
                .with(("persist-credentials", "false")),
        )
        .add_step(
            Step::new("Filesystem and Dependency Vulnerability Scan")
                .uses(
                    "aquasecurity",
                    "trivy-action",
                    "ed142fd0673e97e23eac54620cfb913e5ce36c25",
                )
                .add_with(("scan-type", "fs"))
                .add_with(("scanners", "vuln"))
                .add_with(("vuln-type", "os,library"))
                .add_with(("severity", "HIGH,CRITICAL"))
                .add_with(("ignore-unfixed", "false"))
                .add_with(("exit-code", "1")),
        );

    let draft_release_job = jobs::create_draft_release_job("build");
    let draft_release_pr_job = jobs::create_draft_release_pr_job();
    let events = Event::default()
        .push(Push::default().add_branch("main").add_tag("v*"))
        .pull_request(
            PullRequest::default()
                .add_type(PullRequestType::Opened)
                .add_type(PullRequestType::Synchronize)
                .add_type(PullRequestType::Reopened)
                .add_type(PullRequestType::Labeled)
                .add_branch("main"),
        );
    let build_release_pr_job =
        ReleaseBuilderJob::new("${{ needs.draft_release_pr.outputs.crate_release_name }}")
            .into_job()
            .add_needs("draft_release_pr")
            .cond(Expression::new(
                [
                    "github.event_name == 'pull_request'",
                    "contains(github.event.pull_request.labels.*.name, 'ci: build all targets')",
                ]
                .join(" && "),
            ));
    let build_release_job =
        ReleaseBuilderJob::new("${{ needs.draft_release.outputs.crate_release_name }}")
            .release_id("${{ needs.draft_release.outputs.crate_release_id }}")
            .into_job()
            .add_needs("draft_release")
            .cond(Expression::new(
                [
                    "github.event_name == 'push'",
                    "github.ref == 'refs/heads/main'",
                ]
                .join(" && "),
            ));
    let workflow = Workflow::default()
        .name("ci")
        .add_env(RustFlags::deny("warnings"))
        .on(events)
        .concurrency(Concurrency::default().group("${{ github.workflow }}-${{ github.ref }}"))
        .add_env(("OPENROUTER_API_KEY", "${{secrets.OPENROUTER_API_KEY}}"))
        .add_job("build", build_job)
        .add_job("zsh_rprompt_perf", perf_test_job)
        .add_job("dependency_review", dependency_review_job)
        .add_job("trivy", trivy_job)
        .add_job("draft_release", draft_release_job)
        .add_job("draft_release_pr", draft_release_pr_job)
        .add_job("build_release", build_release_job)
        .add_job("build_release_pr", build_release_pr_job);

    Generate::new(workflow).name("ci.yml").generate().unwrap();
}
