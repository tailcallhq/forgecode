use criterion::{Criterion, criterion_group, criterion_main};
use forge_similarity::{HashOnlyProvider, SimilarityProvider};

fn bench_similarity(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let provider = HashOnlyProvider;

    c.bench_function("similarity/hash_only_compare", |b| {
        b.iter(|| {
            rt.block_on(async {
                provider
                    .compare("agent-1", "implement a Rust HTTP server with TLS")
                    .await
            })
        });
    });
}

criterion_group!(benches, bench_similarity);
criterion_main!(benches);
