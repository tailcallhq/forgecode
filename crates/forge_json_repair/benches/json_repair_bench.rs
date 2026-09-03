use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use forge_json_repair::json_repair;

const BROKEN_SMALL: &str = r#"{"name": "Alice", "age": 30, "active": true"#;

const BROKEN_NESTED: &str = r#"
{
  "users": [
    {"id": 1, "name": "Alice", "tags": ["admin", "user"
    {"id": 2, "name": "Bob"
  ],
  "total": 2
"#;

const MARKDOWN_WRAPPED: &str = r#"
Here is the JSON:
```json
{"key": "value", "list": [1, 2, 3}
```
"#;

fn bench_json_repair(c: &mut Criterion) {
    let mut g = c.benchmark_group("json_repair");

    g.bench_function("small_truncated", |b| {
        b.iter_batched(
            || (),
            |_| json_repair::<serde_json::Value>(BROKEN_SMALL),
            BatchSize::SmallInput,
        );
    });

    g.bench_function("nested_broken", |b| {
        b.iter_batched(
            || (),
            |_| json_repair::<serde_json::Value>(BROKEN_NESTED),
            BatchSize::SmallInput,
        );
    });

    g.bench_function("markdown_wrapped", |b| {
        b.iter_batched(
            || (),
            |_| json_repair::<serde_json::Value>(MARKDOWN_WRAPPED),
            BatchSize::SmallInput,
        );
    });

    g.finish();
}

criterion_group!(benches, bench_json_repair);
criterion_main!(benches);
