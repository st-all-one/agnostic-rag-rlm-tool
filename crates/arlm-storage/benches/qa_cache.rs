//! Benchmarks for the semantic query-answer cache (plan 017).
//!
//! Measures the core cost trade-off: serving a cached answer (HIT, zero LLM)
//! versus persisting a freshly digested answer (the "digest novo" write path),
//! plus staleness-invalidation throughput. Run with `cargo bench -p arlm-storage`.

use arlm_storage::Storage;
use arlm_storage::qa_cache::{StoreAnswerInput, question_hash};
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use tempfile::TempDir;

fn bench_storage() -> (Storage, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let storage = Storage::open(dir.path()).expect("open storage");
    (storage, dir)
}

fn make_input(project: &str, i: usize) -> StoreAnswerInput {
    StoreAnswerInput {
        buffer_id: Some(1),
        project: project.to_string(),
        question_text: format!("question number {i}"),
        question_hash: question_hash(&format!("question number {i}")),
        answer_text: format!("answer number {i}"),
        source_chunk_ids: vec![format!("c{i}")],
        source_hashes: vec![format!("h{i}")],
        model: Some("llama3".to_string()),
        tier_snapshot: Some("{}".to_string()),
        token_count: 12,
    }
}

fn bench_cache_hit_latency(c: &mut Criterion) {
    let (storage, _dir) = bench_storage();
    storage.store_answer(&make_input("p1", 0)).unwrap();
    let qh = question_hash("question number 0");

    c.bench_function("qa_cache/hit_latency", |b| {
        b.iter(|| {
            let row = storage
                .get_cached_answer(black_box("p1"), black_box(&qh))
                .unwrap();
            black_box(row);
        })
    });
}

fn bench_store_digest_cost(c: &mut Criterion) {
    let (storage, _dir) = bench_storage();

    c.bench_function("qa_cache/store_digest_cost", |b| {
        b.iter_batched(
            || make_input("p1", rand_index()),
            |input| {
                let stored = storage.store_answer(black_box(&input)).unwrap();
                black_box(stored);
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_stale_invalidation(c: &mut Criterion) {
    let (storage, _dir) = bench_storage();
    for i in 0..1000 {
        storage.store_answer(&make_input("p1", i)).unwrap();
    }

    c.bench_function("qa_cache/mark_stale_by_hashes_1k", |b| {
        b.iter(|| {
            let n = storage
                .mark_stale_by_hashes(black_box(1), black_box(&["h0".to_string()]))
                .unwrap();
            black_box(n);
        })
    });
}

// Cheap non-crypto varying index to avoid duplicate (project, hash) collisions
// within a single bench run (the reserve-lock would otherwise dedup).
fn rand_index() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(10_000);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

criterion_group!(
    benches,
    bench_cache_hit_latency,
    bench_store_digest_cost,
    bench_stale_invalidation
);
criterion_main!(benches);
