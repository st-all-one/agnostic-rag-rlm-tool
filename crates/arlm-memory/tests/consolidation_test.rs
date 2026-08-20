#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use arlm_memory::consolidation::*;
use arlm_storage::Storage;
use arlm_storage::sqlite::buffers::NewBuffer;
use arlm_storage::sqlite::chunks::NewChunk;
use tempfile::TempDir;

fn setup() -> (ConsolidationEngine, Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    let engine = ConsolidationEngine::new(storage.clone());
    (engine, storage, tmp)
}

fn create_buffer(storage: &Storage) -> i64 {
    storage
        .insert_buffer(&NewBuffer {
            name: "test".to_string(),
            path: "/test".to_string(),
        })
        .unwrap()
}

#[test]
fn test_remove_duplicates() {
    let (engine, storage, _tmp) = setup();
    let buffer_id = create_buffer(&storage);

    let hash = vec![1, 2, 3];
    for _ in 0..3 {
        storage
            .insert_chunk(&NewChunk {
                buffer_id,
                file_path: "src/main.rs".to_string(),
                offset_start: 0,
                offset_end: 100,
                line_start: 1,
                line_end: 10,
                hash: hash.clone(),
                language: None,
                chunk_type: None,
                token_count: None,
            })
            .unwrap();
    }

    let opts = ConsolidateOptions {
        deduplicate: true,
        min_pattern_confidence: 0.0,
    };

    let result = engine.consolidate(buffer_id, &opts).unwrap();
    assert_eq!(result.duplicate_chunks_removed, 2);

    let remaining = storage.count_chunks(buffer_id).unwrap();
    assert_eq!(remaining, 1);
}

#[test]
fn test_remove_low_confidence_patterns() {
    let (engine, storage, _tmp) = setup();
    let buffer_id = create_buffer(&storage);

    storage
        .insert_pattern(Some(buffer_id), None, "high", None, None, Some(0.9))
        .unwrap();
    storage
        .insert_pattern(Some(buffer_id), None, "low", None, None, Some(0.1))
        .unwrap();

    let opts = ConsolidateOptions {
        deduplicate: false,
        min_pattern_confidence: 0.5,
    };

    let result = engine.consolidate(buffer_id, &opts).unwrap();
    assert_eq!(result.low_confidence_patterns_removed, 1);
}

#[test]
fn test_consolidate_empty_project() {
    let (engine, storage, _tmp) = setup();
    let buffer_id = create_buffer(&storage);

    let opts = ConsolidateOptions::default();
    let result = engine.consolidate(buffer_id, &opts).unwrap();
    assert_eq!(result.duplicate_chunks_removed, 0);
    assert_eq!(result.low_confidence_patterns_removed, 0);
}
