use gh_workflow::generate::Generate;
use gh_workflow::toolchain::Component;
use gh_workflow::*;

use crate::jobs;
use crate::steps::setup_protoc;

/// Generate the autofix workflow
pub fn generate_autofix_workflow() {
    let lint_fix_job = Job::new("Lint Fix")
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Checkout Code").uses("actions", "checkout", "v6"))
        .add_step(Step::new("Install SQLite").run("sudo apt-get install -y libsqlite3-dev"))
        .add_step(setup_protoc())
        .add_step(
            Step::toolchain()
                .add_nightly()
                .add_component(Component::Clippy)
                .add_component(Component::Rustfmt),
        )
        .add_step(Step::new("Cargo Fmt").run(jobs::fmt_cmd(true)))
        .add_step(Step::new("Cargo Clippy").run(jobs::clippy_cmd(true)))
        .add_step(
            Step::new("Cargo Clippy String Safety").run(jobs::clippy_string_safety_cmd(false)),
        )
        .add_step(Step::new("Autofix").uses(
            "autofix-ci",
            "action",
            "7a166d7532b277f34e16238930461bf77f9d7ed8",
        ));

    let events = Event::default()
        .push(Push::default().add_branch("main"))
        .pull_request(
            PullRequest::default()
                .add_type(PullRequestType::Opened)
                .add_type(PullRequestType::Synchronize)
                .add_type(PullRequestType::Reopened)
                .add_branch("main"),
        );

    let workflow = Workflow::default()
        .name("autofix.ci")
        .add_env(RustFlags::deny("warnings"))
        .on(events)
        .concurrency(
            Concurrency::default()
                .group("autofix-${{github.ref}}")
                .cancel_in_progress(false),
        )
        .add_job("lint", lint_fix_job);

    Generate::new(workflow)
        .name("autofix.yml")
        .generate()
        .unwrap();
}
