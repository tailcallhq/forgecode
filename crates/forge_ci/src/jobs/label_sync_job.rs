use crate::workflow_model::{Job, Level, Permissions, Step};

/// Create a job to sync GitHub labels
pub fn label_sync_job() -> Job {
    Job::new("label-sync")
        .permissions(Permissions::default().contents(Level::Read).issues(Level::Write))
        .add_step(
                Step::new("Checkout Code").uses("actions", "checkout", "d23441a48e516b6c34aea4fa41551a30e30af803")
        )
        .add_step(
            Step::new("Sync Labels").run(
                "npx -y github-label-sync@3.0.0 \\\n  --access-token ${{ secrets.GITHUB_TOKEN }} \\\n  --labels \".github/labels.json\" \\\n  ${{ github.repository }}"
            )
        )
}
