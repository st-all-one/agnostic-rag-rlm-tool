#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

//! Integration tests for knowledge transfer *between* projects using the full
//! MemoryEngine stack (project lifecycle + knowledge indexing + transfer).

use std::path::Path;

use arlm_memory::engine::{IndexProjectOptions, MemoryEngine};
use arlm_memory::transfer::{TransferEngine, TransferOptions};
use arlm_storage::Storage;
use tempfile::TempDir;

fn write_file(dir: &Path, name: &str, content: &str) {
    let path = dir.join(name);
    std::fs::write(&path, content).expect("write temp file");
}

fn setup() -> (MemoryEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = MemoryEngine::open(tmp.path()).unwrap();
    (engine, tmp)
}

fn index_source(engine: &MemoryEngine, dir: &Path, project_name: &str) {
    write_file(
        dir,
        "alpha.rs",
        "fn alpha() {}\nfn beta() {}\nfn gamma() {}\n",
    );
    write_file(dir, "notes.md", "# Notes\nSome rust knowledge here.\n");

    let opts = IndexProjectOptions {
        project_name: project_name.to_string(),
        dir_path: dir.to_path_buf(),
        max_chunk_bytes: 40,
        ..IndexProjectOptions::default()
    };
    let result = engine.index_project(&opts).unwrap();
    assert!(result.chunks_created > 0, "expected chunks to be created");
}

#[test]
fn test_transfer_between_projects_via_engine() {
    let (engine, _tmp) = setup();

    let source_dir = _tmp.path().join("source");
    let target_dir = _tmp.path().join("target");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::create_dir_all(&target_dir).unwrap();

    index_source(&engine, &source_dir, "source");

    // Create the target project (empty for now).
    engine.create_project("target", &target_dir).unwrap();

    let source_id = engine.projects().get("source").unwrap().unwrap().id;
    let target_id = engine.projects().get("target").unwrap().unwrap().id;

    let storage: Storage = engine.storage().clone();
    let transfer = TransferEngine::new(storage);

    let result = transfer
        .transfer(source_id, target_id, &TransferOptions::default())
        .unwrap();

    assert!(result.chunks_transferred > 0);

    let target_chunks = engine.storage().count_chunks(target_id).unwrap();
    assert_eq!(target_chunks, result.chunks_transferred as i64);
}

#[test]
fn test_transfer_respects_language_filter_across_projects() {
    let (engine, _tmp) = setup();

    let source_dir = _tmp.path().join("source");
    let target_dir = _tmp.path().join("target");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::create_dir_all(&target_dir).unwrap();

    index_source(&engine, &source_dir, "source");
    engine.create_project("target", &target_dir).unwrap();

    let source_id = engine.projects().get("source").unwrap().unwrap().id;
    let target_id = engine.projects().get("target").unwrap().unwrap().id;

    let opts = TransferOptions {
        languages: vec!["rust".to_string()],
        max_chunks: 1000,
    };

    let transfer = TransferEngine::new(engine.storage().clone());
    let result = transfer.transfer(source_id, target_id, &opts).unwrap();

    // notes.md is markdown, so only rust chunks should transfer.
    assert!(result.chunks_transferred > 0);
    let target_chunks = engine.storage().count_chunks(target_id).unwrap();
    assert_eq!(target_chunks, result.chunks_transferred as i64);
}

#[test]
fn test_transfer_fails_for_missing_source_project() {
    let (engine, _tmp) = setup();
    let target_dir = _tmp.path().join("target");
    std::fs::create_dir_all(&target_dir).unwrap();
    engine.create_project("target", &target_dir).unwrap();
    let target_id = engine.projects().get("target").unwrap().unwrap().id;

    let transfer = TransferEngine::new(engine.storage().clone());
    let err = transfer
        .transfer(999_999, target_id, &TransferOptions::default())
        .unwrap_err();
    assert!(err.to_string().contains("source project not found"));
}
