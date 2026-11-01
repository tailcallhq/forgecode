use gh_workflow::*;

/// Create a homebrew release job
pub fn release_homebrew_job() -> Job {
    Job::new("homebrew_release")
        .runs_on(crate::steps::sccache_runner())
        .add_step(
            Step::new("Checkout Code").uses("actions", "checkout", "v6")
                .add_with(("repository", "antinomyhq/homebrew-code-forge"))
                .add_with(("ref", "main"))
                .add_with(("token", "${{ secrets.HOMEBREW_ACCESS }}")),
        )
        // Make script executable and run it with token
        .add_step(
            Step::new("Update Homebrew Formula").run("GITHUB_TOKEN=\"${{ secrets.HOMEBREW_ACCESS }}\" ./update-formula.sh ${{ github.event.release.tag_name }}"),
        )
}
