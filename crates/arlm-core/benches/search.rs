use criterion::{criterion_group, criterion_main, Criterion};
use arlm_core::compaction::{Compaction, SearchResult};
use arlm_core::token_counter::{TokenCounter, get_context_limit};

fn bench_token_counter(c: &mut Criterion) {
    let mut group = c.benchmark_group("token_counter");

    let short_text = "Hello world, this is a test.";
    let long_text = "The quick brown fox jumps over the lazy dog. \
                     This is a longer sentence with many words to estimate tokens. \
                     We are testing the performance of the token counter across \
                     different text lengths and complexities.";

    group.bench_function("estimate_short", |b| {
        b.iter(|| TokenCounter::estimate(short_text));
    });

    group.bench_function("estimate_long", |b| {
        b.iter(|| TokenCounter::estimate(long_text));
    });

    group.bench_function("context_limit_lookup", |b| {
        b.iter(|| get_context_limit("gpt-4o"));
    });

    group.finish();
}

fn bench_compaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("compaction");

    let compaction = Compaction::new(1000);
    let results: Vec<SearchResult> = (0..20)
        .map(|i| SearchResult {
            score: (i as f32) / 20.0,
            content: format!("## Section {i}\nContent for section {i} with some details."),
            file_path: format!("file_{i}.rs"),
        })
        .collect();
    let context = results
        .iter()
        .map(|r| r.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    group.bench_function("compact_20_sections", |b| {
        b.iter(|| compaction.compact(&context, &results));
    });

    group.finish();
}

criterion_group!(benches, bench_token_counter, bench_compaction);
criterion_main!(benches);
