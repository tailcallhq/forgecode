use gh_workflow::generate::Generate;
use gh_workflow::*;
use indexmap::indexmap;
use serde_json::json;

/// Generate the stale issues and PRs workflow
pub fn generate_stale_workflow() {
    let workflow = Workflow::default()
        .name("Close Stale Issues and PR")
        .on(Event::default().add_schedule(Schedule::new("0 * * * *")))
        .permissions(
            Permissions::default()
                .issues(Level::Write)
                .pull_requests(Level::Write),
        )
        .env(Env::from(indexmap! {
            "DAYS_BEFORE_ISSUE_STALE".to_string() => json!("30"),
            "DAYS_BEFORE_ISSUE_CLOSE".to_string() => json!("7"),
            "DAYS_BEFORE_PR_STALE".to_string() => json!("5"),
            "DAYS_BEFORE_PR_CLOSE".to_string() => json!("10"),
        }))
        .add_job(
            "stale",
            Job::new("Stale Issues")
                .runs_on(crate::steps::sccache_runner())
                .add_step(
                    Step::new("Mark Stale Issues").uses("actions", "stale", "v10")
                        .with(Input::from(indexmap! {
                            "stale-issue-label".to_string() => json!("state: inactive"),
                            "stale-pr-label".to_string() => json!("state: inactive"),
                            "stale-issue-message".to_string() => json!(r#"**Action required:** Issue inactive for ${{ env.DAYS_BEFORE_ISSUE_STALE }} days.
Status update or closure in ${{ env.DAYS_BEFORE_ISSUE_CLOSE }} days."#),
                            "close-issue-message".to_string() => json!("Issue closed after ${{ env.DAYS_BEFORE_ISSUE_CLOSE }} days of inactivity."),
                            "stale-pr-message".to_string() => json!(r#"**Action required:** PR inactive for ${{ env.DAYS_BEFORE_PR_STALE }} days.
Status update or closure in ${{ env.DAYS_BEFORE_PR_CLOSE }} days."#),
                            "close-pr-message".to_string() => json!("PR closed after ${{ env.DAYS_BEFORE_PR_CLOSE }} days of inactivity."),
                            "days-before-issue-stale".to_string() => json!("${{ env.DAYS_BEFORE_ISSUE_STALE }}"),
                            "days-before-issue-close".to_string() => json!("${{ env.DAYS_BEFORE_ISSUE_CLOSE }}"),
                            "days-before-pr-stale".to_string() => json!("${{ env.DAYS_BEFORE_PR_STALE }}"),
                            "days-before-pr-close".to_string() => json!("${{ env.DAYS_BEFORE_PR_CLOSE }}"),
                        })),
                ),
        );

    Generate::new(workflow)
        .name("stale.yml")
        .generate()
        .unwrap();
}
