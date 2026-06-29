use criterion::{Criterion, criterion_group, criterion_main};
use forge_stream::MpscStream;
use futures::StreamExt;

fn bench_mpsc_stream(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut g = c.benchmark_group("stream");

    g.bench_function("mpsc_stream/1000_items", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut stream = MpscStream::spawn(|tx| async move {
                    for i in 0u32..1000 {
                        let _ = tx.send(i).await;
                    }
                });
                let mut count = 0u32;
                while stream.next().await.is_some() {
                    count += 1;
                }
                count
            })
        });
    });

    g.finish();
}

criterion_group!(benches, bench_mpsc_stream);
criterion_main!(benches);
