//! dhat heap profiling harness for forge_json_repair.
//!
//! Run with:
//!   cargo run --example heap_profile --features dhat-heap
//!
//! Output: dhat-heap.json — open with https://nnethercote.github.io/dh_view/dh_view.html

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    // Alloc-heavy path: repeated parse of a deeply nested broken JSON doc.
    let broken = r#"
    {
      "agents": [
        {"id": 1, "name": "Alice", "tasks": ["write code", "review PR"
        {"id": 2, "name": "Bob", "tasks": ["test", "deploy"
        {"id": 3, "name": "Carol"
      ],
      "meta": {"version": 2, "created": "2026-06-28"
    "#;

    for _ in 0..1_000 {
        let _: Result<serde_json::Value, _> = forge_json_repair::json_repair(broken);
    }

    println!("heap_profile: 1000 iterations complete");
    // _profiler drops here → writes dhat-heap.json
}
