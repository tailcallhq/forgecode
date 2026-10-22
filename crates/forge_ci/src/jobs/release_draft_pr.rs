use gh_workflow::*;
use indexmap::indexmap;

/// Create a draft release job for GitHub Actions that runs on PRs
pub fn create_draft_release_pr_job() -> Job {
    Job::new("draft_release_pr")
        .runs_on(crate::steps::sccache_runner())
        .name("Draft Release for PR")
        .cond(Expression::new(
            "github.event_name == 'pull_request' && contains(github.event.pull_request.labels.*.name, 'ci: build all targets')",
        ))
        .add_step(Step::new("Checkout Code").uses("actions", "checkout", "v6"))
        .add_step(
            Step::new("Set Release Version").run(
                r#"echo "crate_release_name=pr-build-${{ github.event.number }}" >> $GITHUB_OUTPUT && echo "crate_release_id=pr-build-${{ github.event.number }}" >> $GITHUB_OUTPUT"#,
            )
            .id("set_output"),
        )
        .outputs(indexmap! {
            "crate_release_name".to_string() => "${{ steps.set_output.outputs.crate_release_name }}".to_string(),
            "crate_release_id".to_string() => "${{ steps.set_output.outputs.crate_release_id }}".to_string()
        })
}
