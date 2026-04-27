use std::path::PathBuf;

use tracing::debug;
use tracing_appender::non_blocking::{self, WorkerGuard};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{self, Layer, filter};

pub fn init_tracing(log_path: PathBuf) -> anyhow::Result<Guard> {
    debug!(path = %log_path.display(), "Initializing logging system in JSON format");

    let (writer, guard, level) = prepare_writer(log_path);

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
) -> (
    non_blocking::NonBlocking,
    WorkerGuard,
    tracing_subscriber::EnvFilter,
) {
    let append = tracing_appender::rolling::daily(log_path, "forge.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(append);
    (
        non_blocking,
        guard,
        tracing_subscriber::EnvFilter::new("forge=debug"),
    )
}

pub struct Guard(#[allow(dead_code)] WorkerGuard);
