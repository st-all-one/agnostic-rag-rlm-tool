#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

use arlm_search::context::{build_context, build_search_results, load_chunks};
use arlm_search::types::{HybridResult, OutputFormat};
use arlm_storage::Storage;
use arlm_storage::sqlite::buffers::NewBuffer;
use arlm_storage::sqlite::chunks::NewChunk;
use tempfile::TempDir;

fn setup() -> (Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (storage, tmp)
}

fn create_test_data(storage: &Storage) -> (i64, i64) {
    let buf_id = storage
        .insert_buffer(&NewBuffer {
            name: "test".to_string(),
            path: "/test".to_string(),
        })
        .unwrap();

    let chunk_id = storage
        .insert_chunk(&NewChunk {
            buffer_id: buf_id,
            file_path: "src/main.rs".to_string(),
            offset_start: 0,
            offset_end: 100,
            line_start: 1,
            line_end: 10,
            hash: vec![0u8],
            language: Some("rust".to_string()),
            chunk_type: None,
            token_count: Some(50),
        })
        .unwrap();

    storage
        .insert_chunk_content(chunk_id, "fn main() { println!(\"hello\"); }")
        .unwrap();

    (buf_id, chunk_id)
}

#[test]
fn test_build_context_prompt() {
    let (storage, _tmp) = setup();
    let (_, chunk_id) = create_test_data(&storage);

    let results = vec![HybridResult {
        chunk_id,
        score: 0.85,
        is_summary: false,
    }];

    let ctx = build_context(&storage, &results, OutputFormat::Prompt, None).unwrap();
    assert!(ctx.contains("## Project Context"));
    assert!(ctx.contains("src/main.rs"));
    assert!(ctx.contains("fn main()"));
    assert!(ctx.contains("0.85"));
}

#[test]
fn test_build_context_markdown() {
    let (storage, _tmp) = setup();
    let (_, chunk_id) = create_test_data(&storage);

    let results = vec![HybridResult {
        chunk_id,
        score: 0.90,
        is_summary: false,
    }];

    let ctx = build_context(&storage, &results, OutputFormat::Markdown, None).unwrap();
    assert!(ctx.contains("# Search Results"));
    assert!(ctx.contains("src/main.rs"));
}

#[test]
fn test_build_context_json() {
    let (storage, _tmp) = setup();
    let (_, chunk_id) = create_test_data(&storage);

    let results = vec![HybridResult {
        chunk_id,
        score: 0.75,
        is_summary: false,
    }];

    let ctx = build_context(&storage, &results, OutputFormat::Json, None).unwrap();
    assert!(ctx.contains("chunk_id"));
    assert!(ctx.contains("src/main.rs"));
}

#[test]
fn test_build_search_results() {
    let (storage, _tmp) = setup();
    let (_, chunk_id) = create_test_data(&storage);

    let results = vec![HybridResult {
        chunk_id,
        score: 0.95,
        is_summary: false,
    }];

    let search_results = build_search_results(&storage, &results, None).unwrap();
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0].chunk_id, chunk_id);
    assert_eq!(search_results[0].file_path, "src/main.rs");
    assert_eq!(search_results[0].language, Some("rust".to_string()));
}

#[test]
fn test_build_context_empty() {
    let (storage, _tmp) = setup();
    let ctx = build_context(&storage, &[], OutputFormat::Prompt, None).unwrap();
    assert!(ctx.contains("## Project Context"));
}

#[test]
fn test_load_chunks_missing() {
    let (storage, _tmp) = setup();
    let results = vec![HybridResult {
        chunk_id: 999,
        score: 0.5,
        is_summary: false,
    }];

    let chunks = load_chunks(&storage, &results).unwrap();
    assert!(chunks.is_empty());
}
