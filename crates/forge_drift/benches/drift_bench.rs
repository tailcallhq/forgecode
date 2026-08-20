use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use forge_drift::{DriftConfig, DriftDetector, DriftIndex};

fn bench_drift_observe(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let index = Arc::new(DriftIndex::new());
    let detector = DriftDetector::new(DriftConfig::default(), index.clone(), None);

    // Pre-populate the index with a baseline prompt
    let baseline = "implement a Rust HTTP server with rustls and tokio";
    index.observe("agent-1", baseline);

    let mut g = c.benchmark_group("drift");

    g.bench_function("observe_exact_match", |b| {
        b.iter(|| {
            rt.block_on(async { detector.observe("agent-1", baseline, "default", 1000).await })
        });
    });

    g.bench_function("observe_similar_prompt", |b| {
        let similar = "build an HTTP server using Rust with tokio and rustls TLS";
        b.iter(|| {
            rt.block_on(async { detector.observe("agent-1", similar, "default", 2000).await })
        });
    });

    g.bench_function("observe_disjoint_prompt", |b| {
        let disjoint = "write a Python data pipeline with pandas and dask";
        b.iter(|| {
            rt.block_on(async { detector.observe("agent-1", disjoint, "default", 3000).await })
        });
    });

    g.finish();
}

criterion_group!(benches, bench_drift_observe);
criterion_main!(benches);
