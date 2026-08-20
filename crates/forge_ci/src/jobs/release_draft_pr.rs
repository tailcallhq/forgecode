use crate::workflow_model::{Job, Step};

/// Create a draft release job for GitHub Actions that runs on PRs
pub fn create_draft_release_pr_job() -> Job {
    Job::new("Draft Release for PR")
        .if_condition(
            "github.event_name == 'pull_request' && contains(github.event.pull_request.labels.*.name, 'ci: build all targets')",
        )
        .add_step(Step::new("Checkout Code").uses("actions", "checkout", "d23441a48e516b6c34aea4fa41551a30e30af803"))
        .add_step(
            Step::new("Set Release Version").run(
                r#"echo "crate_release_name=pr-build-${{ github.event.number }}" >> "$GITHUB_OUTPUT" && echo "crate_release_id=pr-build-${{ github.event.number }}" >> "$GITHUB_OUTPUT""#,
            )
                .id("set_output"),
        )
        .output("crate_release_name", "${{ steps.set_output.outputs.crate_release_name }}")
        .output("crate_release_id", "${{ steps.set_output.outputs.crate_release_id }}")
}
