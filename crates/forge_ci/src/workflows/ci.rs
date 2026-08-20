use crate::jobs::{self, ReleaseBuilderJob};
use crate::steps::setup_protoc;
use crate::workflow_model::{Event, Job, Level, Permissions, Push, Step, Workflow};

pub fn generate_ci_workflow() {
    let build_job = Job::new("Build and Test")
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Checkout Code").uses(
            "actions",
            "checkout",
            "d23441a48e516b6c34aea4fa41551a30e30af803",
        ))
        .add_step(setup_protoc())
        .add_step(
            Step::new("Setup Rust Toolchain")
                .uses(
                    "actions-rust-lang",
                    "setup-rust-toolchain",
                    "166cdcfd11aee3cb47222f9ddb555ce30ddb9659",
                )
                .input("toolchain", "stable"),
        )
        .add_step(Step::new("Install cargo-llvm-cov").run("cargo install cargo-llvm-cov"))
        .add_step(
            Step::new("Generate coverage")
                .run("cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info"),
        );
    let perf_test_job = Job::new("Performance: zsh rprompt")
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Checkout Code").uses(
            "actions",
            "checkout",
            "d23441a48e516b6c34aea4fa41551a30e30af803",
        ))
        .add_step(setup_protoc())
        .add_step(
            Step::new("Setup Rust Toolchain")
                .uses(
                    "actions-rust-lang",
                    "setup-rust-toolchain",
                    "166cdcfd11aee3cb47222f9ddb555ce30ddb9659",
                )
                .input("toolchain", "stable"),
        )
        .add_step(
            Step::new("Run performance benchmark")
                .run("./scripts/benchmark.sh --threshold 60 zsh rprompt"),
        );
    let draft_release_job = jobs::create_draft_release_job("build");
    let draft_release_pr_job = jobs::create_draft_release_pr_job();
    let build_release_pr_job = ReleaseBuilderJob::new("${{ needs.draft_release_pr.outputs.crate_release_name }}")
        .into_job().needs("draft_release_pr")
        .if_condition("github.event_name == 'pull_request' && contains(github.event.pull_request.labels.*.name, 'ci: build all targets')");
    let build_release_job =
        ReleaseBuilderJob::new("${{ needs.draft_release.outputs.crate_release_name }}")
            .release_id("${{ needs.draft_release.outputs.crate_release_id }}")
            .into_job()
            .needs("draft_release")
            .if_condition("github.event_name == 'push' && github.ref == 'refs/heads/main'");
    let events = Event::default()
        .push(Push::default().add_branch("main").add_tag("v*"))
        .pull_request(["opened", "synchronize", "reopened", "labeled"], ["main"]);
    let workflow = Workflow::new("ci")
        .env("RUSTFLAGS", "-Dwarnings")
        .env("OPENROUTER_API_KEY", "${{secrets.OPENROUTER_API_KEY}}")
        .on(events)
        .concurrency("${{ github.workflow }}-${{ github.ref }}", false)
        .permissions(Permissions::default().contents(Level::Read))
        .add_job("build", build_job)
        .add_job("zsh_rprompt_perf", perf_test_job)
        .add_job("draft_release", draft_release_job)
        .add_job("draft_release_pr", draft_release_pr_job)
        .add_job("build_release", build_release_job)
        .add_job("build_release_pr", build_release_pr_job);
    super::generate_private_workflow(workflow, "ci.yml");
}
