use gh_workflow::*;
use serde_json::Value;

/// Create an NPM release job using matrix strategy for multiple repositories
pub fn release_npm_job() -> Job {
    let matrix = create_npm_matrix();

    Job::new("npm_release")
        .runs_on(crate::steps::sccache_runner())
        .strategy(Strategy { fail_fast: None, max_parallel: None, matrix: Some(matrix) })
        .add_step(
            Step::new("Checkout Code")
                .uses("actions", "checkout", "v6")
                .add_with(("repository", "${{ matrix.repository }}"))
                .add_with(("ref", "main"))
                .add_with(("token", "${{ secrets.NPM_ACCESS }}")),
        )
        // The sccache runner image does not ship Node.js.
        .add_step(crate::steps::setup_node())
        // Make script executable and run it with token
        .add_step(
            Step::new("Update NPM Package")
                .run("./update-package.sh ${{ github.event.release.tag_name }}")
                .add_env(("AUTO_PUSH", "true"))
                .add_env(("CI", "true"))
                .add_env(("NPM_TOKEN", "${{ secrets.NPM_TOKEN }}")),
        )
}

/// Creates a matrix Value for NPM repositories
fn create_npm_matrix() -> Value {
    serde_json::json!({
        "repository": [
            "antinomyhq/npm-code-forge",
            "antinomyhq/npm-forgecode"
        ]
    })
}
