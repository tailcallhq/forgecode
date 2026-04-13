use std::path::PathBuf;

use tracing::debug;
use tracing_appender::non_blocking::{self, WorkerGuard};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{self, Layer, filter};

use crate::Tracker;
use crate::can_track::can_track;

pub fn init_tracing(log_path: PathBuf, tracker: Tracker) -> anyhow::Result<Guard> {
    debug!(path = %log_path.display(), "Initializing logging system in JSON format");

    // If tracking is enabled, use PostHog for logging; otherwise, use a rolling
    // file appender.
    let (writer, guard, level) = prepare_writer(log_path, tracker);

    // Create a filter that only allows logs from forge_ modules
    let filter = filter::filter_fn(|metadata| metadata.target().starts_with("forge_"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_thread_ids(false)
        .with_target(false)
        .with_file(true)
        .with_line_number(true)
        .with_writer(writer)
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_env("FORGE_LOG").unwrap_or(level))
        .with(fmt_layer)
        .init();

    Ok(Guard(guard))
}

fn prepare_writer(
    log_path: PathBuf,
    tracker: Tracker,
) -> (
    non_blocking::NonBlocking,
    WorkerGuard,
    tracing_subscriber::EnvFilter,
) {
    let ((non_blocking, guard), env) = if can_track() {
        let append = PostHogWriter::new(tracker);
        (
            tracing_appender::non_blocking(append),
            tracing_subscriber::EnvFilter::new("forge=info"),
        )
    } else {
        let append = tracing_appender::rolling::daily(log_path, "forge.log");
        (
            tracing_appender::non_blocking(append),
            tracing_subscriber::EnvFilter::new("forge=debug"),
        )
    };
    (non_blocking, guard, env)
}

pub struct Guard(#[allow(dead_code)] WorkerGuard);

struct PostHogWriter {
    tracker: Tracker,
    runtime: tokio::runtime::Runtime,
}

impl PostHogWriter {
    pub fn new(tracker: Tracker) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(1)
            .build()
            .expect("Failed to create Tokio runtime");
        Self { tracker, runtime }
    }
}

impl std::io::Write for PostHogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let tracker = self.tracker.clone();
        let event_kind = crate::EventKind::Trace(buf.to_vec());
        self.runtime.spawn(async move {
            let _ = tracker.dispatch(event_kind).await;
        });
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
