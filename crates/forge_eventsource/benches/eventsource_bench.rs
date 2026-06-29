use criterion::{Criterion, criterion_group, criterion_main};
use forge_eventsource_stream::EventStream;
use futures::TryStreamExt;

fn bench_event_stream_parse(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    // 1000 SSE events in one chunk
    let payload: String = (0..1000)
        .map(|i| format!("data: event payload number {i}\n\n"))
        .collect();

    let mut g = c.benchmark_group("eventsource");

    g.bench_function("parse_1000_events_single_chunk", |b| {
        b.iter(|| {
            rt.block_on(async {
                let chunk = Ok::<_, std::convert::Infallible>(payload.clone());
                EventStream::new(futures::stream::once(async move { chunk }))
                    .try_collect::<Vec<_>>()
                    .await
                    .expect("parse ok")
                    .len()
            })
        });
    });

    // 100 events, each in its own chunk (simulates real streaming)
    g.bench_function("parse_100_events_fragmented", |b| {
        let chunks: Vec<String> = (0..100)
            .map(|i| format!("data: fragment event {i}\n\n"))
            .collect();

        b.iter(|| {
            rt.block_on(async {
                let stream = futures::stream::iter(
                    chunks
                        .iter()
                        .map(|s| Ok::<_, std::convert::Infallible>(s.clone())),
                );
                EventStream::new(stream)
                    .try_collect::<Vec<_>>()
                    .await
                    .expect("parse ok")
                    .len()
            })
        });
    });

    g.finish();
}

criterion_group!(benches, bench_event_stream_parse);
criterion_main!(benches);
