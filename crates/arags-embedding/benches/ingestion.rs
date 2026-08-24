#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use arags_embedding::embedder::fallback::FallbackEmbedder;
use arags_embedding::pipeline::{IngestOptions, IngestionPipeline, discover_files};
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_ingestion(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingestion");

    // Create a temp directory with some test files
    let tmp = tempfile::tempdir().expect("tempdir");
    for i in 0..20 {
        let path = tmp.path().join(format!("file_{i}.rs"));
        let content = format!(
            "// File {i}\nfn main() {{\n    println!(\"hello from file {i}\");\n    let x = {i} * 2;\n    println!(\"x = {{x}}\");\n}}\n"
        );
        std::fs::write(&path, content).expect("write");
    }

    let embedder = Arc::new(FallbackEmbedder::new(128));
    let pipeline = IngestionPipeline::new(embedder, None);
    let options = IngestOptions::default();

    group.bench_function("ingest_20_files", |b| {
        b.iter(|| {
            let files = discover_files(tmp.path(), &[], &[]).expect("discover");
            pipeline.ingest(&files, &options).expect("ingest")
        });
    });

    group.finish();
}

criterion_group!(benches, bench_ingestion);
criterion_main!(benches);
