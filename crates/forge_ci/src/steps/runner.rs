use gh_workflow::*;

/// Label of the self-hosted, autoscaled Cloud Run runner pool (no Docker
/// daemon). Only this label: the pool's scaler uses `noDefaultLabels`, so a job
/// that also asked for `self-hosted` would never get an instance started.
pub const SCCACHE_RUNNER: &str = "sccache";

/// `runs-on` value for jobs on the sccache pool, rendered as a block list.
pub fn sccache_runner() -> Vec<&'static str> {
    vec![SCCACHE_RUNNER]
}

/// Installs Node.js; the sccache runner image does not ship it.
pub fn setup_node() -> Step<Use> {
    Step::new("Setup Node")
        .uses("actions", "setup-node", "v4")
        .add_with(("node-version", "20"))
}
