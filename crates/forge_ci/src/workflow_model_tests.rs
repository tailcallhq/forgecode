use crate::workflow_model::{Event, Job, Level, Permissions, Push, Step, Workflow};

#[test]
fn serializes_an_ordered_workflow_with_github_actions_keys() {
    let fixture = Workflow::new("CI")
        .on(Event::default().push(Push::default().add_branch("main")))
        .permissions(Permissions::default().contents(Level::Read))
        .add_job(
            "check",
            Job::new("Check").add_step(Step::new("Checkout").uses(
                "actions",
                "checkout",
                "d23441a48e516b6c34aea4fa41551a30e30af803",
            )),
        );

    let actual = fixture.to_yaml().unwrap();
    let expected = "name: CI\non:\n  push:\n    branches:\n      - main\npermissions:\n  contents: read\njobs:\n  check:\n    name: Check\n    runs-on: ubuntu-latest\n    steps:\n      - name: Checkout\n        uses: actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803\n";

    let actual = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&actual).unwrap();
    let expected = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(expected).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn serializes_job_permissions_for_label_synchronization() {
    let fixture = Workflow::new("Labels").add_job(
        "label-sync",
        Job::new("label-sync")
            .permissions(Permissions::default().issues(Level::Write))
            .add_step(Step::new("Checkout").uses(
                "actions",
                "checkout",
                "d23441a48e516b6c34aea4fa41551a30e30af803",
            )),
    );

    let actual = fixture.to_yaml().unwrap();
    assert!(actual.contains("issues: write"));
}

#[test]
fn serializes_release_drafter_event_branches_and_token_environment() {
    let fixture = Workflow::new("Release Drafter")
        .on(Event::default().pull_request_target(
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
        .add_job(
            "update_release_draft",
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
            ),
        );

    let actual = fixture.to_yaml().unwrap();
    assert!(actual.contains("pull_request_target:"));
    assert!(actual.contains("branches:"));
    assert!(actual.contains("GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}"));
}
