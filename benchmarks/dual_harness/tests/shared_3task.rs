//! FR-DH-001: forgecode shared-3task.v1 process_smoke (no model serving).

use std::path::PathBuf;

use dual_harness::{load_fixture, run_forgecode_fixture};

fn fixture_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DUAL_HARNESS_FIXTURE") {
        return PathBuf::from(p);
    }
    dual_harness::default_shared_3task_path()
}

#[tokio::test]
async fn shared_3task_all_pass() {
    let path = fixture_path();
    assert!(
        path.exists(),
        "fixture missing at {} (set DUAL_HARNESS_FIXTURE)",
        path.display()
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    // SAFETY: test process; isolate workdir for t2.
    unsafe {
        std::env::set_var("DUAL_HARNESS_WORKDIR", tmp.path());
    }

    let fixture = load_fixture(&path).expect("load fixture");
    let outcomes = run_forgecode_fixture(&fixture).await.expect("run fixture");
    assert_eq!(outcomes.len(), 3, "expected 3 shared tasks");
    for o in &outcomes {
        assert!(o.passed, "{} failed: {}", o.task_id, o.detail);
    }
}
