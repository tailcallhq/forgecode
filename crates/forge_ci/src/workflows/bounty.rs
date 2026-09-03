use crate::workflow_model::{Event, Job, Level, Permissions, Step, Workflow};

/// Generate the bounty management workflow (v2).
///
/// Two jobs cover the full bounty lifecycle:
/// - `sync-all-issues`: fetches all open issues with any bounty label and
///   reconciles their label sets in one pass. Triggered on label/assignment
///   events and daily on a schedule.
/// - `sync-pr`: propagates bounty value labels from linked issues to the PR on
///   open/edit, and applies the rewarded lifecycle on merge.
pub fn generate_bounty_workflow() {
    let events = Event::default()
        .pull_request(["opened", "edited", "reopened"], std::iter::empty::<&str>())
        .pull_request_target(["closed"], std::iter::empty::<&str>())
        .issues(["assigned", "unassigned", "labeled", "unlabeled"])
        .schedule("0 2 * * *");

    let workflow = Workflow::new("Bounty Management")
        .on(events)
        .permissions(
            Permissions::default()
                .issues(Level::Read)
                .pull_requests(Level::Read),
        )
        .add_job(
            "sync-all-issues",
            Job::new("Sync all bounty issues")
                .permissions(Permissions::default().issues(Level::Write))
                .add_step(Step::new("Checkout").uses(
                    "actions",
                    "checkout",
                    "d23441a48e516b6c34aea4fa41551a30e30af803",
                ))
                .add_step(Step::new("Install npm packages").run(
                    "npm ci --ignore-scripts --no-audit --no-fund",
                ))
                .add_step(Step::new("Sync all bounty labels").run(
                    "npx -y tsx@4.20.6 .github/scripts/bounty/src/sync-all-issues.ts --repo ${{ github.repository }} --token ${{ secrets.GITHUB_TOKEN }} --execute",
                )),
        )
        .add_job(
            "sync-pr",
            Job::new("Sync PR bounty labels")
                .if_condition(
                    "github.event_name == 'pull_request' || github.event_name == 'pull_request_target'",
                )
                .permissions(
                    Permissions::default().issues(Level::Write).pull_requests(Level::Write),
                )
                .add_step(Step::new("Checkout").uses(
                    "actions",
                    "checkout",
                    "d23441a48e516b6c34aea4fa41551a30e30af803",
                ))
                .add_step(Step::new("Install npm packages").run(
                    "npm ci --ignore-scripts --no-audit --no-fund",
                ))
                .add_step(Step::new("Sync bounty labels").run(
                    "npx -y tsx@4.20.6 .github/scripts/bounty/src/sync-pr.ts --pr ${{ github.event.pull_request.number }} --repo ${{ github.repository }} --token ${{ secrets.GITHUB_TOKEN }} --execute",
                )),
        );

    super::generate_private_workflow(workflow, "bounty.yml");
}
