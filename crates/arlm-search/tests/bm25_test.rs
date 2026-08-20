#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

use arlm_search::bm25::Bm25Search;
use arlm_storage::sqlite::buffers::NewBuffer;
use arlm_storage::sqlite::chunks::NewChunk;
use arlm_storage::Storage;
use tempfile::TempDir;

fn setup() -> (Bm25Search, Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    let search = Bm25Search::new(&storage).unwrap();
    (search, storage, tmp)
}

fn create_buffer(storage: &Storage, idx: u32) -> i64 {
    storage
        .insert_buffer(&NewBuffer {
            name: format!("test-{idx}"),
            path: format!("/test-{idx}"),
        })
        .unwrap()
}

fn create_chunk(storage: &Storage, buffer_id: i64, file_path: &str) -> i64 {
    storage
        .insert_chunk(&NewChunk {
            buffer_id,
            file_path: file_path.to_string(),
            offset_start: 0,
            offset_end: 100,
            line_start: 1,
            line_end: 10,
            hash: vec![0u8],
            language: Some("rust".to_string()),
            chunk_type: None,
            token_count: Some(50),
        })
        .unwrap()
}

#[test]
fn test_populate_and_search() {
    let (search, storage, _tmp) = setup();
    let buffer_id = create_buffer(&storage, 0);
    let chunk_id = create_chunk(&storage, buffer_id, "src/main.rs");

    search
        .insert_into_fts(chunk_id, "fn main() { println!(\"hello\"); }")
        .unwrap();

    let results = search.search("hello", buffer_id, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, chunk_id);
}

#[test]
fn test_search_no_match() {
    let (search, _storage, _tmp) = setup();
    let results = search.search("nonexistent", 1, 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_search_buffer_filter() {
    let (search, storage, _tmp) = setup();
    let buf1 = create_buffer(&storage, 0);
    let buf2 = create_buffer(&storage, 1);

    let c1 = create_chunk(&storage, buf1, "a.rs");
    let c2 = create_chunk(&storage, buf2, "b.rs");

    search.insert_into_fts(c1, "alpha bravo").unwrap();
    search.insert_into_fts(c2, "alpha charlie").unwrap();

    let results = search.search("alpha", buf1, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, c1);
}

#[test]
fn test_search_all() {
    let (search, storage, _tmp) = setup();
    let buf = create_buffer(&storage, 0);
    let c1 = create_chunk(&storage, buf, "a.rs");
    let c2 = create_chunk(&storage, buf, "b.rs");

    search.insert_into_fts(c1, "hello world").unwrap();
    search.insert_into_fts(c2, "hello rust").unwrap();

    let results = search.search_all("hello", 10).unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn test_populate_fts() {
    let (search, storage, _tmp) = setup();
    let buf = create_buffer(&storage, 0);
    let c1 = create_chunk(&storage, buf, "a.rs");
    let c2 = create_chunk(&storage, buf, "b.rs");

    storage.insert_chunk_content(c1, "foo bar").unwrap();
    storage.insert_chunk_content(c2, "baz qux").unwrap();

    let count = search.populate_fts().unwrap();
    assert_eq!(count, 2);

    let results = search.search("foo", buf, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, c1);
}
