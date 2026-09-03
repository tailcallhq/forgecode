//! CLI entry for forgecode dual-harness shared-3task smoker (FR-DH-001).

use std::path::PathBuf;
use std::process::ExitCode;

use dual_harness::{default_shared_3task_path, load_fixture, run_forgecode_fixture};

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::var_os("DUAL_HARNESS_WORKDIR").is_none() {
        let tmp = std::env::temp_dir().join(format!("dual-harness-forge-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        // SAFETY: single-threaded before spawn; set for child processes.
        unsafe { std::env::set_var("DUAL_HARNESS_WORKDIR", &tmp) };
        eprintln!("DUAL_HARNESS_WORKDIR defaulted to {}", tmp.display());
    }

    let path = std::env::var_os("DUAL_HARNESS_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(default_shared_3task_path);

    if !path.exists() {
        eprintln!("FAIL: fixture missing at {}", path.display());
        return ExitCode::from(2);
    }

    let fixture = match load_fixture(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("FAIL: {e}");
            return ExitCode::from(2);
        }
    };

    let outcomes = match run_forgecode_fixture(&fixture).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("FAIL: {e}");
            return ExitCode::from(1);
        }
    };

    let mut failed = 0usize;
    for o in &outcomes {
        println!(
            "{} {}: {}",
            if o.passed { "PASS" } else { "FAIL" },
            o.task_id,
            o.detail
        );
        if !o.passed {
            failed += 1;
        }
    }
    if failed > 0 {
        return ExitCode::from(1);
    }
    println!(
        "OK: {} tasks (forgecode / {})",
        outcomes.len(),
        fixture.fixture_id
    );
    ExitCode::SUCCESS
}
