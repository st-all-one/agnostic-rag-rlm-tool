#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use arags_memory::transfer::*;
use arags_storage::Storage;
use arags_storage::sqlite::buffers::NewBuffer;
use arags_storage::sqlite::chunks::NewChunk;
use tempfile::TempDir;

fn setup() -> (TransferEngine, Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    let engine = TransferEngine::new(storage.clone());
    (engine, storage, tmp)
}

fn create_buffer(storage: &Storage, name: &str) -> i64 {
    storage
        .insert_buffer(&NewBuffer {
            name: name.to_string(),
            path: format!("/tmp/{name}"),
        })
        .unwrap()
}

#[test]
fn test_transfer_chunks() {
    let (engine, storage, _tmp) = setup();
    let from_id = create_buffer(&storage, "source");
    let to_id = create_buffer(&storage, "target");

    storage
        .insert_chunk(&NewChunk {
            buffer_id: from_id,
            file_path: "a.rs".to_string(),
            offset_start: 0,
            offset_end: 50,
            line_start: 1,
            line_end: 5,
            hash: vec![1],
            language: Some("rust".to_string()),
            chunk_type: None,
            token_count: Some(10),
        })
        .unwrap();

    let opts = TransferOptions::default();
    let result = engine.transfer(from_id, to_id, &opts).unwrap();
    assert_eq!(result.chunks_transferred, 1);

    let target_chunks = storage.count_chunks(to_id).unwrap();
    assert_eq!(target_chunks, 1);
}

#[test]
fn test_transfer_with_language_filter() {
    let (engine, storage, _tmp) = setup();
    let from_id = create_buffer(&storage, "source");
    let to_id = create_buffer(&storage, "target");

    storage
        .insert_chunk(&NewChunk {
            buffer_id: from_id,
            file_path: "a.rs".to_string(),
            offset_start: 0,
            offset_end: 50,
            line_start: 1,
            line_end: 5,
            hash: vec![1],
            language: Some("rust".to_string()),
            chunk_type: None,
            token_count: None,
        })
        .unwrap();

    storage
        .insert_chunk(&NewChunk {
            buffer_id: from_id,
            file_path: "b.py".to_string(),
            offset_start: 0,
            offset_end: 50,
            line_start: 1,
            line_end: 5,
            hash: vec![2],
            language: Some("python".to_string()),
            chunk_type: None,
            token_count: None,
        })
        .unwrap();

    let opts = TransferOptions {
        languages: vec!["rust".to_string()],
        max_chunks: 100,
    };

    let result = engine.transfer(from_id, to_id, &opts).unwrap();
    assert_eq!(result.chunks_transferred, 1);
}

#[test]
fn test_transfer_max_chunks_limit() {
    let (engine, storage, _tmp) = setup();
    let from_id = create_buffer(&storage, "source");
    let to_id = create_buffer(&storage, "target");

    for i in 0..10 {
        storage
            .insert_chunk(&NewChunk {
                buffer_id: from_id,
                file_path: format!("f{i}.rs"),
                offset_start: 0,
                offset_end: 50,
                line_start: 1,
                line_end: 5,
                hash: vec![i as u8],
                language: None,
                chunk_type: None,
                token_count: None,
            })
            .unwrap();
    }

    let opts = TransferOptions {
        languages: Vec::new(),
        max_chunks: 3,
    };

    let result = engine.transfer(from_id, to_id, &opts).unwrap();
    assert_eq!(result.chunks_transferred, 3);
}
