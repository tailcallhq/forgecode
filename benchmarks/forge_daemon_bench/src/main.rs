// forge_daemon_bench — Measures daemon vs fork+exec throughput at M=8/16/32
//
// Methodology:
//   - fork+exec baseline: tokio::process::Command::new("true") × M parallel
//   - daemon dispatch: forge_daemon::DaemonDispatch::dispatch("true", ...) × M parallel
//   - Measures: wall-clock time per batch, agents/s, latency per task
//
// "true" is used as the forge_bin so the benchmark is self-contained and
// measures process-launch overhead only (no LLM wait).
//
// Run: cargo run --release --bin forge_daemon_bench -- [--m 32] [--iters 5]
//
// Results are printed to stdout in a tabular format.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use forge_daemon::DaemonDispatch;
use tokio::sync::Semaphore;

const DEFAULT_ITERS: usize = 5;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "warn".into())
                .as_str(),
        )
        .init();

    // Parse --m and --iters from args.
    let args: Vec<String> = std::env::args().collect();
    let m_values: Vec<usize> = {
        let mut ms = vec![];
        let mut i = 1;
        while i < args.len() {
            if args[i] == "--m" && i + 1 < args.len() {
                ms.push(args[i + 1].parse().unwrap_or(32));
                i += 2;
            } else {
                i += 1;
            }
        }
        if ms.is_empty() { vec![8, 16, 32] } else { ms }
    };
    let iters: usize = {
        let mut it = DEFAULT_ITERS;
        let mut i = 1;
        while i < args.len() {
            if args[i] == "--iters" && i + 1 < args.len() {
                it = args[i + 1].parse().unwrap_or(DEFAULT_ITERS);
                i += 2;
            } else {
                i += 1;
            }
        }
        it
    };

    println!("forge-daemon benchmark (M={m_values:?}, iters={iters})");
    println!("forge_bin = /usr/bin/true  (measures spawn overhead, no LLM wait)");
    println!();
    println!(
        "{:<6}  {:<12}  {:<14}  {:<12}  {:<14}  {:<10}",
        "M", "fork+exec(ms)", "fork+exec(a/s)", "daemon(ms)", "daemon(a/s)", "speedup×"
    );
    println!("{}", "-".repeat(80));

    for &m in &m_values {
        let fork_ms = bench_fork_exec(m, iters).await?;
        let daemon_ms = bench_daemon_dispatch(m, iters)?;

        let fork_as = m as f64 / (fork_ms / 1000.0);
        let daemon_as = m as f64 / (daemon_ms / 1000.0);
        let speedup = daemon_as / fork_as;

        println!(
            "{:<6}  {:<12.1}  {:<14.1}  {:<12.1}  {:<14.1}  {:<10.2}",
            m, fork_ms, fork_as, daemon_ms, daemon_as, speedup
        );
    }

    println!();
    println!("Notes:");
    println!("  fork+exec baseline: tokio::process::Command::new(\"/usr/bin/true\") × M");
    println!("  daemon dispatch:    forge_daemon::DaemonDispatch (Zig posix_spawn via C ABI)");
    println!("  Speedup shows process-launch overhead reduction only.");
    println!("  Real forge workloads have ~47ms extra dyld+tokio init per spawn (#74).");

    Ok(())
}

/// Baseline: launch /usr/bin/true M times in parallel with tokio::process::Command.
async fn bench_fork_exec(m: usize, iters: usize) -> Result<f64> {
    let mut total = Duration::ZERO;

    for _ in 0..iters {
        let sem = Arc::new(Semaphore::new(m));
        let mut handles = Vec::with_capacity(m);

        let start = Instant::now();
        for _ in 0..m {
            let permit = sem.clone().acquire_owned().await?;
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                tokio::process::Command::new("/usr/bin/true")
                    .output()
                    .await
                    .ok();
            }));
        }
        for h in handles {
            h.await.ok();
        }
        total += start.elapsed();
    }

    Ok(total.as_secs_f64() * 1000.0 / iters as f64)
}

/// Daemon dispatch: run /usr/bin/true via forge_daemon C-ABI (posix_spawn path).
fn bench_daemon_dispatch(m: usize, iters: usize) -> Result<f64> {
    let mut total = Duration::ZERO;
    let cwd = std::env::current_dir()?;

    for _ in 0..iters {
        let start = Instant::now();

        // Run M tasks sequentially via the Zig C-ABI hot path.
        // The posix_spawn overhead is what we're measuring; thread parallelism
        // is outside the scope of this benchmark (daemon handles concurrency).
        for _ in 0..m {
            let _ = DaemonDispatch::dispatch("/usr/bin/true", "bench-prompt", "bench-model", &cwd)?;
        }

        total += start.elapsed();
    }

    Ok(total.as_secs_f64() * 1000.0 / iters as f64)
}
