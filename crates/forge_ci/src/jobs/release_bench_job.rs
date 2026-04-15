use gh_workflow::*;

/// Create a benchmark job that runs terminal-bench via Modal on release.
///
/// The job runs after the build_release job completes and downloads the
/// `x86_64-unknown-linux-musl` binary from the release before dispatching
/// the benchmark suite through `uvx harbor`.
pub fn release_bench_job() -> Job {
    Job::new("bench")
        .runs_on("ubuntu-latest")
        .add_step(Step::new("Checkout Code").uses("actions", "checkout", "v6"))
        .add_step(
            Step::new("Install uv")
                .uses("astral-sh", "setup-uv", "v5")
                .add_with(("enable-cache", "false")),
        )
        .add_step(
            Step::new("Download forge musl binary")
                .run("gh release download ${{ github.event.release.tag_name }} --pattern forge-x86_64-unknown-linux-musl --output forge && chmod +x forge")
                .add_env(("GH_TOKEN", "${{ secrets.GITHUB_TOKEN }}")),
        )
        .add_step(
            Step::new("Run Benchmarks")
                .run(
                    "uvx harbor run \
                      -e modal \
                      -d terminal-bench@2.0 \
                      --agent-import-path bench.forge_agent:ForgeAgent \
                      --timeout-multiplier 2.0 \
                      --export-traces \
                      --export-verifier-metadata \
                      --force-build \
                      -n 32",
                )
                .add_env(("FORGE_BIN", "${{ github.workspace }}/forge"))
                .add_env(("FORGE_OVERRIDE_PROVIDER", "open_router"))
                .add_env(("FORGE_OVERRIDE_MODEL", "z-ai/glm-5.1"))
                .add_env(("OPENROUTER_API_KEY", "${{ secrets.OPENROUTER_API_KEY }}"))
                .add_env(("MODAL_TOKEN_ID", "${{ secrets.MODAL_TOKEN_ID }}"))
                .add_env(("MODAL_TOKEN_SECRET", "${{ secrets.MODAL_TOKEN_SECRET }}")),
        )
}
