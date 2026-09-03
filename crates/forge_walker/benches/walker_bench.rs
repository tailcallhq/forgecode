use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use forge_walker::Walker;

fn bench_walk_tempdir(c: &mut Criterion) {
    // Build a modest temp tree to walk.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    for i in 0..20 {
        let sub = root.join(format!("dir_{i}"));
        std::fs::create_dir_all(&sub).unwrap();
        for j in 0..10 {
            std::fs::write(sub.join(format!("file_{j}.txt")), b"hello forge").unwrap();
        }
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    c.bench_function("walker/walk_200_files", |b| {
        b.iter(|| {
            rt.block_on(async {
                Walker::min_all()
                    .cwd(PathBuf::from(root))
                    .get()
                    .await
                    .expect("walk ok")
            })
        });
    });
}

criterion_group!(benches, bench_walk_tempdir);
criterion_main!(benches);
