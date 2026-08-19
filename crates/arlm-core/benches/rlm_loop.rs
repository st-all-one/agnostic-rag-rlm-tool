use criterion::{criterion_group, criterion_main, Criterion};
use arlm_core::engine::EngineState;
use arlm_core::budget::RunBudget;
use arlm_core::cache::ResultCache;
use arlm_core::router::DepthRouter;

fn bench_engine_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine_state");

    group.bench_function("next_node_id", |b| {
        let state = EngineState::new();
        b.iter(|| state.next_node_id());
    });

    group.bench_function("record_visit", |b| {
        let state = EngineState::new();
        b.iter(|| state.record_visit(0));
    });

    group.finish();
}

fn bench_budget(c: &mut Criterion) {
    let mut group = c.benchmark_group("budget");

    group.bench_function("budget_check", |b| {
        let budget = RunBudget::new(1.0, 100_000, 5, 300_000);
        b.iter(|| budget.check());
    });

    group.finish();
}

fn bench_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache");

    group.bench_function("cache_lookup_miss", |b| {
        let cache = ResultCache::default();
        b.iter(|| cache.get("some task that is not cached", "test"));
    });

    group.bench_function("cache_insert_and_lookup", |b| {
        let cache = ResultCache::default();
        b.iter(|| {
            cache.put("task", "test", "result");
            cache.get("task", "test")
        });
    });

    group.finish();
}

fn bench_depth_router(c: &mut Criterion) {
    let mut group = c.benchmark_group("depth_router");

    group.bench_function("suggest_depth", |b| {
        let mut router = DepthRouter::new();
        router.record_outcome(0, true);
        router.record_outcome(1, true);
        router.record_outcome(2, false);
        b.iter(|| router.suggest_depth("analyze this codebase"));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_engine_state,
    bench_budget,
    bench_cache,
    bench_depth_router
);
criterion_main!(benches);
