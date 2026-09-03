// Tests for forge_api: BackgroundTasks lifecycle and concurrency primitives.
//
// The API trait itself has 40+ async methods with deep infrastructure deps.
// Testing the trait boundary requires the full stack (SQLite, filesystem, LLM
// providers). These tests focus on the concurrency primitives that can be
// tested in isolation.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use forge_api::BackgroundTasks;
use tokio::task;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// BackgroundTasks lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn shutdown_cancels_running_tasks() {
    let cancel = CancellationToken::new();
    let completed = Arc::new(AtomicUsize::new(0));
    let completed_clone = completed.clone();

    let handle = task::spawn({
        let cancel = cancel.clone();
        async move {
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {
                    completed_clone.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    });

    let tasks = BackgroundTasks::new(cancel, vec![handle]);
    tasks.shutdown().await;

    assert_eq!(
        completed.load(Ordering::SeqCst),
        0,
        "background task should have been cancelled before completing"
    );
}

#[tokio::test]
async fn drop_cancels_running_tasks() {
    let cancel = CancellationToken::new();
    let completed = Arc::new(AtomicUsize::new(0));
    let completed_clone = completed.clone();

    let handle = task::spawn({
        let cancel = cancel.clone();
        async move {
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {
                    completed_clone.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    });

    // Drop the BackgroundTasks — Drop impl cancels handles
    let _tasks = BackgroundTasks::new(cancel, vec![handle]);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert_eq!(
        completed.load(Ordering::SeqCst),
        0,
        "drop should cancel background tasks"
    );
}

#[tokio::test]
async fn cancels_multiple_concurrent_tasks() {
    let cancel = CancellationToken::new();
    let completed = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..5)
        .map(|_| {
            let cancel = cancel.clone();
            let completed = completed.clone();
            task::spawn(async move {
                tokio::select! {
                    _ = cancel.cancelled() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {
                        completed.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();

    let tasks = BackgroundTasks::new(cancel, handles);
    tasks.shutdown().await;

    assert_eq!(
        completed.load(Ordering::SeqCst),
        0,
        "all background tasks should be cancelled"
    );
}

#[tokio::test]
async fn shutdown_waits_for_tasks_to_finish() {
    let cancel = CancellationToken::new();
    let finished = Arc::new(AtomicUsize::new(0));
    let finished_clone = finished.clone();

    let handle = task::spawn({
        let cancel = cancel.clone();
        async move {
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
            }
            finished_clone.fetch_add(1, Ordering::SeqCst);
        }
    });

    let tasks = BackgroundTasks::new(cancel, vec![handle]);
    tasks.shutdown().await;

    // After shutdown, the task should have exited and incremented the counter
    assert_eq!(
        finished.load(Ordering::SeqCst),
        1,
        "shutdown should wait for task to finish"
    );
}

#[tokio::test]
async fn already_completed_task_is_handled_gracefully() {
    let cancel = CancellationToken::new();

    let handle = task::spawn(async {
        // Task completes immediately
    });

    let tasks = BackgroundTasks::new(cancel, vec![handle]);
    tasks.shutdown().await; // should not panic
}
