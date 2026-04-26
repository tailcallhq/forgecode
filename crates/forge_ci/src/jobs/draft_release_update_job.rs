use gh_workflow::*;

/// Create a job to update the release draft
pub fn draft_release_update_job() -> Job {
    Job::new("update_release_draft")
        .add_step(
            Step::new("Auto Labeler")
                .uses("release-drafter", "release-drafter/autolabeler", "v7")
                .if_condition(Expression::new(
                    "github.event_name == 'pull_request_target'",
                ))
                .env(("GITHUB_TOKEN", "${{ secrets.GITHUB_TOKEN }}"))
                .add_with(("config-name", "release-drafter.yml")),
        )
        .add_step(
            Step::new("Release Drafter")
                .uses("release-drafter", "release-drafter", "v7")
                .env(("GITHUB_TOKEN", "${{ secrets.GITHUB_TOKEN }}"))
                .add_with(("config-name", "release-drafter.yml")),
        )
}
