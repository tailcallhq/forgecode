use crate::workflow_model::{Event, Job, Level, Permissions, Push, Step, Workflow};

/// Generate labels workflow.
pub fn generate_labels_workflow() {
    let labels_workflow = Workflow::new("Github Label Sync")
        .on(Event::default().push(Push::default().add_branch("main")))
        .permissions(
            Permissions::default()
                .contents(Level::Read)
                .issues(Level::Read),
        )
        .add_job(
            "label-sync",
            Job::new("label-sync")
                .permissions(Permissions::default().issues(Level::Write))
                .add_step(Step::new("Checkout").uses(
                    "actions",
                    "checkout",
                    "d23441a48e516b6c34aea4fa41551a30e30af803",
                ))
                .add_step(Step::new("Sync labels").run(
                    r#"npx -y github-label-sync@3.0.0 \
  --access-token ${{ secrets.GITHUB_TOKEN }} \
  --labels ".github/labels.json" \
  ${{ github.repository }}"#,
                )),
        );

    super::generate_private_workflow(labels_workflow, "labels.yml");
}
