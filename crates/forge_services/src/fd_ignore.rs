use std::sync::Arc;

use anyhow::Context;
use forge_domain::IgnorePatternsRepository;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use tokio::sync::OnceCell;
use tracing::warn;

/// Lazily-loaded, process-lifetime cache of the server's gitignore patterns.
///
/// Encapsulates the infrastructure handle so callers don't have to hold onto
/// the raw `Arc<F>` themselves.
pub(crate) struct ServerIgnoreMatcher<F> {
    infra: Arc<F>,
    cell: OnceCell<Option<Gitignore>>,
}

impl<F> ServerIgnoreMatcher<F> {
    pub(crate) fn new(infra: Arc<F>) -> Self {
        Self { infra, cell: OnceCell::new() }
    }
}

impl<F: IgnorePatternsRepository> ServerIgnoreMatcher<F> {
    /// Returns the compiled matcher, loading and caching it on first use.
    ///
    /// Returns `None` when the server is unreachable or the response cannot be
    /// compiled; a warning is logged and the caller is expected to proceed
    /// without filtering.
    pub(crate) async fn get(&self) -> Option<&Gitignore> {
        self.cell
            .get_or_init(|| async {
                match self
                    .infra
                    .list_ignore_patterns()
                    .await
                    .and_then(|contents| build_matcher(&contents))
                {
                    Ok(gi) => Some(gi),
                    Err(err) => {
                        warn!(error = ?err, "failed to load server ignore patterns; continuing without");
                        None
                    }
                }
            })
            .await
            .as_ref()
    }
}

/// Compiles a [`Gitignore`] from the raw contents of the server's
/// `ignore_patterns.txt` using the same semantics as the server: builder root
/// `/` (non-anchored globs match absolute and relative paths alike), blank /
/// `#`-prefixed lines skipped.
fn build_matcher(contents: &str) -> anyhow::Result<Gitignore> {
    let mut builder = GitignoreBuilder::new("/");
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        builder
            .add_line(None, line)
            .with_context(|| format!("invalid ignore pattern: {line}"))?;
    }
    builder.build().context("failed to build ignore matcher")
}
