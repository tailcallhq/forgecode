use criterion::{Criterion, criterion_group, criterion_main};
use forge_fs::ForgeFS;

fn bench_forge_fs(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    // Create a temp file to read.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench_target.txt");
    std::fs::write(&path, "a".repeat(64 * 1024)).unwrap(); // 64 KiB

    let mut g = c.benchmark_group("forge_fs");

    g.bench_function("read_64kib", |b| {
        b.iter(|| rt.block_on(async { ForgeFS::read(path.as_path()).await.expect("read ok") }));
    });

    g.bench_function("write_then_read_64kib", |b| {
        let content = "b".repeat(64 * 1024);
        let write_path = dir.path().join("write_bench.txt");
        b.iter(|| {
            rt.block_on(async {
                ForgeFS::write(write_path.as_path(), &content)
                    .await
                    .expect("write ok")
            })
        });
    });

    g.finish();
}

criterion_group!(benches, bench_forge_fs);
criterion_main!(benches);
