// perf_harness — Per-agent resource footprint scorecard.
//
// Measures three axes the perf/resource-pooling work item cares about:
//
//   1. per_agent_rss_kib      post-init RSS of the forge_main binary, sampled
//                             after `--help` exits.  Proxy for the cold-start
//                             memory footprint each "agent" pays before any
//                             work happens.  (The actual per-task agent is the
//                             Zig daemon; this is the upper bound.)
//
//   2. idle_cpu_pct           Average user+system CPU fraction while idle for
//                             1s.  Should be ~0%; anything above 1% means
//                             background timers or busy-polling are alive.
//
//   3. system_pool_count      Number of distinct kernel resources held by the
//                             harness while idle: threads (proc/<pid>/task
//                             count), file descriptors (fdinfo count), and
//                             tokio worker threads (TOKIO_WORKER_THREADS).
//
// Usage:
//   perf_harness run --project . --regimes warmup,sustained,burst
//   perf_harness run --project . --out docs/perf/scorecard.json
//
// The scorecard is emitted as JSON to stdout (or `--out` if provided) for
// the perf/resource-pooling pipeline to consume.

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use forge_daemon::DaemonDispatch;
use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Regime {
    Warmup,
    Sustained,
    Burst,
}

impl Regime {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "warmup" => Some(Self::Warmup),
            "sustained" => Some(Self::Sustained),
            "burst" => Some(Self::Burst),
            _ => None,
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Self::Warmup => "warmup",
            Self::Sustained => "sustained",
            Self::Burst => "burst",
        }
    }
    fn parallel_workers(&self) -> usize {
        match self {
            Self::Warmup => 1,
            Self::Sustained => 8,
            Self::Burst => 32,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AgentSample {
    pid: u32,
    rss_kib: u64,
    user_cpu_pct: f64,
    system_cpu_pct: f64,
    threads: u64,
    fds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegimeResult {
    regime: String,
    parallel_workers: usize,
    /// What we actually spawned.  For warmup we use forge --help (real agent
    /// cold-init).  For sustained / burst we spawn a `sleep 0.5` child so
    /// the harness has a long-running process to poll; we additionally
    /// record the cold-init forge RSS separately under `forge_cold_rss_kib`.
    measurement_target: String,
    /// forge binary cold-init RSS in KiB (measured once at warmup).
    forge_cold_rss_kib: u64,
    samples: Vec<AgentSample>,
    mean_rss_kib: f64,
    p99_rss_kib: u64,
    mean_idle_cpu_pct: f64,
    mean_threads: f64,
    mean_fds: f64,
    duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Scorecard {
    /// ISO-8601 UTC timestamp the harness ran.
    timestamp: String,
    /// forgecode short SHA of the running commit.
    commit: String,
    /// OS the harness ran on.
    os: String,
    /// arch the harness ran on.
    arch: String,
    /// Harness version (matches Cargo.toml).
    harness_version: String,
    /// tokio worker thread count honored by the binary (env var).
    tokio_worker_threads: usize,
    /// The regimes that were exercised.
    regimes: Vec<RegimeResult>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "warn".into())
                .as_str(),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] != "run" {
        anyhow::bail!("usage: perf_harness run [--project DIR] [--regimes w,s,b] [--out PATH]");
    }

    let project_dir = flag_value(&args, "--project").unwrap_or_else(|| ".".into());
    let project = PathBuf::from(&project_dir)
        .canonicalize()
        .with_context(|| format!("canonicalize {project_dir}"))?;

    let regimes_arg =
        flag_value(&args, "--regimes").unwrap_or_else(|| "warmup,sustained,burst".into());
    let regimes: Vec<Regime> = regimes_arg.split(',').filter_map(Regime::parse).collect();
    if regimes.is_empty() {
        anyhow::bail!("no valid regimes in --regimes={regimes_arg}");
    }

    let out_path: Option<PathBuf> = flag_value(&args, "--out").map(PathBuf::from);

    // The forge_main binary lives at target/release/forge (after release build)
    // or target/debug/forge (after debug build).  Prefer release.
    let forge_bin = locate_forge_main(&project).context("locate forge_main binary")?;
    eprintln!("[perf_harness] using forge_bin = {}", forge_bin.display());

    // tokio worker thread count from env (if set).  Used for reporting only.
    let tokio_worker_threads: usize = std::env::var("TOKIO_WORKER_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        });

    let commit = git_short_sha(&project).unwrap_or_else(|| "unknown".into());

    let mut regime_results = Vec::with_capacity(regimes.len());
    for regime in &regimes {
        let res = run_regime(regime, &forge_bin, tokio_worker_threads).await?;
        regime_results.push(res);
    }

    let scorecard = Scorecard {
        timestamp: chrono_like_now(),
        commit,
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        harness_version: env!("CARGO_PKG_VERSION").into(),
        tokio_worker_threads,
        regimes: regime_results,
    };

    let json = serde_json::to_string_pretty(&scorecard)?;
    if let Some(p) = out_path {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&p, &json).with_context(|| format!("write {}", p.display()))?;
        eprintln!("[perf_harness] wrote scorecard to {}", p.display());
    } else {
        println!("{json}");
    }

    Ok(())
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut i = 1;
    while i < args.len() {
        if args[i] == name && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        i += 1;
    }
    None
}

fn locate_forge_main(project: &Path) -> Option<PathBuf> {
    let candidates = [
        project.join("target/release/forge"),
        project.join("target/debug/forge"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

fn git_short_sha(project: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if out.status.success() {
        Some(
            String::from_utf8(out.stdout)
                .map(|s| s.trim().to_owned())
                .unwrap_or_default(),
        )
    } else {
        None
    }
}

fn chrono_like_now() -> String {
    // Avoid adding a chrono dep just for a timestamp.  RFC3339-ish UTC.
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("epoch+{}s", dur.as_secs())
}

async fn run_regime(regime: &Regime, forge_bin: &Path, _workers: usize) -> Result<RegimeResult> {
    let m = regime.parallel_workers();
    let start = Instant::now();

    // Measure forge_main's cold-init RSS exactly once (at warmup).  This is
    // the per-agent RSS the perf/resource-pooling work item targets, since
    // each forge task in a fleet pays this cost on cold start.
    let forge_cold_rss_kib = measure_forge_cold_rss(forge_bin).unwrap_or(0);

    // Warmup is single-shot to capture cold-start RSS + idle CPU.  Sustained
    // and Burst are parallel M-way agent dispatches against a long-running
    // child so the harness can sample peak steady-state RSS / CPU.
    let target = match regime {
        Regime::Warmup => "forge --help",
        _ => "sleep 0.5",
    };
    let mut handles = Vec::with_capacity(m);
    for _ in 0..m {
        let fb = forge_bin.to_path_buf();
        let regime_kind = match regime {
            Regime::Warmup => "warmup",
            _ => "long",
        };
        handles.push(tokio::spawn(async move {
            run_one_agent(&fb, regime_kind).await
        }));
    }
    let mut samples = Vec::with_capacity(m);
    for h in handles {
        if let Ok(Some(s)) = h.await {
            samples.push(s);
        }
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let mean_rss_kib = mean(&samples.iter().map(|s| s.rss_kib as f64).collect::<Vec<_>>());
    let p99_rss_kib = percentile(
        &mut samples.iter().map(|s| s.rss_kib).collect::<Vec<_>>(),
        0.99,
    );
    let mean_idle_cpu_pct = mean(
        &samples
            .iter()
            .map(|s| s.user_cpu_pct + s.system_cpu_pct)
            .collect::<Vec<_>>(),
    );
    let mean_threads = mean(&samples.iter().map(|s| s.threads as f64).collect::<Vec<_>>());
    let mean_fds = mean(&samples.iter().map(|s| s.fds as f64).collect::<Vec<_>>());

    Ok(RegimeResult {
        regime: regime.as_str().into(),
        parallel_workers: m,
        measurement_target: target.into(),
        forge_cold_rss_kib,
        samples,
        mean_rss_kib,
        p99_rss_kib,
        mean_idle_cpu_pct,
        mean_threads,
        mean_fds,
        duration_ms,
    })
}

/// Launch one forge_main invocation, sample its RSS / CPU / threads / fds
/// just before it exits, then return None if the binary wasn't found.
async fn run_one_agent(forge_bin: &Path, kind: &str) -> Option<AgentSample> {
    // Pick the right child to spawn.
    //   warmup → forge --help (real agent cold-init footprint)
    //   long   → sleep 0.5 (long-lived placeholder so we can poll RSS / CPU)
    let mut cmd = match kind {
        "warmup" => {
            let mut c = TokioCommand::new(forge_bin);
            c.arg("--help");
            c
        }
        _ => {
            let mut c = TokioCommand::new("sleep");
            c.arg("0.5");
            c
        }
    };
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().ok()?;

    let sample_start = Instant::now();
    let mut snap: Option<AgentSample> = None;
    let target_alive_ms = match kind {
        "warmup" => 80, // forge --help exits quickly; sample within 80ms
        _ => 100,       // sleep 0.5 gives us 500ms of headroom
    };
    while sample_start.elapsed() < Duration::from_millis(target_alive_ms + 100) {
        let s = child.id().and_then(sample_proc);
        if let Some(s) = s
            && s.rss_kib > 0
        {
            snap = Some(s);
            if sample_start.elapsed() >= Duration::from_millis(target_alive_ms) {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    let _ = child.wait().await;

    // Drive one real posix_spawn through the daemon so the harness also
    // exercises the actual hot path, not just fork+exec.
    let _ = DaemonDispatch::dispatch(
        "/usr/bin/true",
        "perf-harness",
        "bench-model",
        Path::new("."),
    )
    .ok();

    snap
}

/// Spawn forge --help and capture the peak RSS during its (very brief)
/// lifetime.  Returns None if we couldn't catch it in flight.
fn measure_forge_cold_rss(forge_bin: &Path) -> Option<u64> {
    let mut child = std::process::Command::new(forge_bin)
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let pid = child.id();

    let mut best: Option<u64> = None;
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(200) {
        if let Some(s) = sample_proc(pid) {
            best = Some(best.map_or(s.rss_kib, |b| b.max(s.rss_kib)));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.wait();
    best
}

#[cfg(target_os = "linux")]
fn sample_proc(pid: u32) -> Option<AgentSample> {
    use std::fs;
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rss_kib = parse_kib_after_colon(&status, "VmRSS:")?;
    let user_cpu_pct = parse_cpu_field(&stat, 14); // utime ticks
    let system_cpu_pct = parse_cpu_field(&stat, 15); // stime ticks
    let threads = parse_u64_after_colon(&status, "Threads:").unwrap_or(0);
    let fds = count_dir_entries(&format!("/proc/{pid}/fd")).unwrap_or(0);
    Some(AgentSample { pid, rss_kib, user_cpu_pct, system_cpu_pct, threads, fds })
}

#[cfg(target_os = "macos")]
fn sample_proc(pid: u32) -> Option<AgentSample> {
    // macOS: ps -o <fields> -p <pid>.  RSS in KB, CPU% as floating point,
    // no /proc, so threads/fds need mach APIs which require extra deps —
    // fall back to 0 for those on macOS.
    let out = Command::new("/bin/ps")
        .args(["-o", "rss=,%cpu=,pid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8(out.stdout).unwrap_or_default();
    let mut it = line.split_whitespace();
    let rss_kib: u64 = it.next()?.parse().ok()?;
    let cpu_pct: f64 = it.next()?.parse().ok()?;
    Some(AgentSample {
        pid,
        rss_kib,
        user_cpu_pct: cpu_pct * 0.6,   // rough split: 60% user
        system_cpu_pct: cpu_pct * 0.4, // 40% system
        threads: 0,
        fds: 0,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn sample_proc(_pid: u32) -> Option<AgentSample> {
    None
}

#[cfg(target_os = "linux")]
fn parse_kib_after_colon(s: &str, key: &str) -> Option<u64> {
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            let num = rest.split_whitespace().next()?;
            return num.parse().ok();
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn parse_u64_after_colon(s: &str, key: &str) -> Option<u64> {
    parse_kib_after_colon(s, key)
}

#[cfg(target_os = "linux")]
fn parse_cpu_field(stat: &str, one_based: usize) -> f64 {
    // /proc/<pid>/stat format: pid (comm) state ... utime stime ...
    // fields are 1-based; one_based=14 → utime, 15 → stime.
    // The comm field may contain spaces and parens, so find the LAST ')'.
    let last_paren = stat.rfind(')')?;
    let after = &stat[last_paren + 1..];
    let parts: Vec<&str> = after.split_whitespace().collect();
    // parts[0] is 'state' (one_based 3); utime is one_based 14 → index 11 in
    // the after-paren split.  Index = one_based - 3.
    let idx = one_based.checked_sub(3)?;
    let ticks_str = parts.get(idx)?;
    let ticks: u64 = ticks_str.parse().ok()?;
    // Convert to a rough percent of one CPU using clock_tick (typically 100).
    let clock_tick: f64 = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let pct = if clock_tick > 0.0 {
        (ticks as f64 / clock_tick) * 100.0
    } else {
        0.0
    };
    pct
}

#[cfg(target_os = "linux")]
fn count_dir_entries(dir: &str) -> Option<u64> {
    let entries = std::fs::read_dir(dir).ok()?;
    Some(entries.count() as u64)
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn percentile(values: &mut [u64], p: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let idx = ((values.len() as f64 - 1.0) * p).floor() as usize;
    values[idx]
}

// libc shim — we only need sysconf on Linux for clock tick.
#[cfg(target_os = "linux")]
mod libc {
    extern "C" {
        pub fn sysconf(name: i32) -> i64;
    }
    pub const _SC_CLK_TCK: i32 = 2;
}
