use crate::workflow_model::{Event, Job, Level, Permissions, Step, Workflow};

/// Generate the stale issues and PRs workflow
pub fn generate_stale_workflow() {
    let workflow = Workflow::new("Close Stale Issues and PR")
        .on(Event::default().schedule("0 * * * *"))
        .permissions(
            Permissions::default()
                .issues(Level::Write)
                .pull_requests(Level::Write),
        )
        .env("DAYS_BEFORE_ISSUE_STALE", "30")
        .env("DAYS_BEFORE_ISSUE_CLOSE", "7")
        .env("DAYS_BEFORE_PR_STALE", "5")
        .env("DAYS_BEFORE_PR_CLOSE", "10")
        .add_job(
            "stale",
            Job::new("Stale Issues")
                .add_step(
                    Step::new("Mark Stale Issues")
                        .uses("actions", "stale", "1e223db275d687790206a7acac4d1a11bd6fe629")
                        .input("stale-issue-label", "state: inactive")
                        .input("stale-pr-label", "state: inactive")
                        .input("stale-issue-message", r#"**Action required:** Issue inactive for ${{ env.DAYS_BEFORE_ISSUE_STALE }} days.
Status update or closure in ${{ env.DAYS_BEFORE_ISSUE_CLOSE }} days."#)
                        .input("close-issue-message", "Issue closed after ${{ env.DAYS_BEFORE_ISSUE_CLOSE }} days of inactivity.")
                        .input("stale-pr-message", r#"**Action required:** PR inactive for ${{ env.DAYS_BEFORE_PR_STALE }} days.
Status update or closure in ${{ env.DAYS_BEFORE_PR_CLOSE }} days."#)
                        .input("close-pr-message", "PR closed after ${{ env.DAYS_BEFORE_PR_CLOSE }} days of inactivity.")
                        .input("days-before-issue-stale", "${{ env.DAYS_BEFORE_ISSUE_STALE }}")
                        .input("days-before-issue-close", "${{ env.DAYS_BEFORE_ISSUE_CLOSE }}")
                        .input("days-before-pr-stale", "${{ env.DAYS_BEFORE_PR_STALE }}")
                        .input("days-before-pr-close", "${{ env.DAYS_BEFORE_PR_CLOSE }}"),
                ),
        );

    super::generate_private_workflow(workflow, "stale.yml");
}
