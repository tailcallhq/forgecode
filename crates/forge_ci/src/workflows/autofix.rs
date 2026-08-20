use crate::jobs;
use crate::workflow_model::{Event, Job, Level, Permissions, Push, Step, Workflow};

/// Generate the autofix workflow
pub fn generate_autofix_workflow() {
    let lint_fix_job = Job::new("Lint Fix")
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Checkout Code").uses(
            "actions",
            "checkout",
            "d23441a48e516b6c34aea4fa41551a30e30af803",
        ))
        .add_step(Step::new("Install SQLite").run("sudo apt-get install -y libsqlite3-dev"))
        .add_step(
            Step::new("Setup Protobuf Compiler")
                .uses(
                    "arduino",
                    "setup-protoc",
                    "c65c819552d16ad3c9b72d9dfd5ba5237b9c906b",
                )
                .input("repo-token", "${{ secrets.GITHUB_TOKEN }}"),
        )
        .add_step(
            Step::new("Setup Rust Toolchain")
                .uses(
                    "actions-rust-lang",
                    "setup-rust-toolchain",
                    "166cdcfd11aee3cb47222f9ddb555ce30ddb9659",
                )
                .input("components", "clippy, rustfmt"),
        )
        .add_step(Step::new("Cargo Clippy").run(jobs::clippy_cmd(false)))
        .add_step(
            Step::new("Cargo Clippy String Safety").run(jobs::clippy_string_safety_cmd(false)),
        );

    let events = Event::default()
        .push(Push::default().add_branch("main"))
        .pull_request(["opened", "synchronize", "reopened"], ["main"]);

    let workflow = Workflow::new("autofix.ci")
        .env("RUSTFLAGS", "-Dwarnings")
        .on(events)
        .permissions(Permissions::default().contents(Level::Read))
        .concurrency("autofix-${{github.ref}}", false)
        .add_job("lint", lint_fix_job);

    super::generate_private_workflow(workflow, "autofix.yml");
}
