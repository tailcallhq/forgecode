use crate::jobs::draft_release_update_job;
use crate::workflow_model::{Event, Level, Permissions, Push, Workflow};

/// Generate release drafter workflow
pub fn generate_release_drafter_workflow() {
    let release_drafter = Workflow::new("Release Drafter")
        .on(Event::default()
            .push(Push::default().add_branch("main"))
            .pull_request_target(
                [
                    "opened",
                    "reopened",
                    "synchronize",
                    "labeled",
                    "unlabeled",
                    "closed",
                ],
                ["main"],
            ))
        .permissions(
            Permissions::default()
                .contents(Level::Read)
                .pull_requests(Level::Read),
        )
        .add_job(
            "update_release_draft",
            draft_release_update_job().permissions(
                Permissions::default()
                    .contents(Level::Write)
                    .pull_requests(Level::Read),
            ),
        );

    super::generate_private_workflow(release_drafter, "release-drafter.yml");
}
