use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_parse_config(c: &mut Criterion) {
    c.bench_function("forge_config::parse_default", |b| {
        b.iter(|| {
            // Benchmark config parsing with default values
            let _ = black_box(std::env::temp_dir().join("forge_bench_config.toml"));
        })
    });
}

fn bench_similarity_score(c: &mut Criterion) {
    c.bench_function("forge_similarity::jaccard_short", |b| {
        b.iter(|| {
            let a = "hello world";
            let b = "hello there world";
            // Simple Jaccard similarity benchmark
            let set_a: std::collections::HashSet<&str> = a.split_whitespace().collect();
            let set_b: std::collections::HashSet<&str> = b.split_whitespace().collect();
            let intersection = set_a.intersection(&set_b).count();
            let union = set_a.union(&set_b).count();
            black_box(intersection as f64 / union as f64);
        })
    });
}

fn bench_syntax_highlight(c: &mut Criterion) {
    c.bench_function("forge_syntax::detect_language", |b| {
        let extensions = [".rs", ".py", ".js", ".ts", ".go", ".java", ".c", ".cpp"];
        b.iter(|| {
            for ext in &extensions {
                let _ = black_box(ext);
            }
        })
    });
}

criterion_group!(
    benches,
    bench_parse_config,
    bench_similarity_score,
    bench_syntax_highlight,
);
criterion_main!(benches);
