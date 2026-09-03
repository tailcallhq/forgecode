use crate::workflow_model::{Job, Step};

/// Create a job to update the release draft
pub fn draft_release_update_job() -> Job {
    Job::new("update_release_draft").add_step(
        Step::new("Release Drafter")
            .uses(
                "release-drafter",
                "release-drafter",
                "5a60cd8ddda6dc14fce77159675b8fd2cdca4007",
            )
            .input("config-name", "release-drafter.yml")
            .input("version", crate::jobs::FORK_RELEASE_VERSION)
            .env("GITHUB_TOKEN", "${{ secrets.GITHUB_TOKEN }}"),
    )
}
