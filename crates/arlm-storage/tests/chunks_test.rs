#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss, clippy::cast_possible_wrap, clippy::cast_lossless, clippy::float_cmp)]

use arlm_storage::sqlite::chunks::NewChunk;
use arlm_storage::Storage;
use tempfile::TempDir;

fn setup_storage() -> (Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (storage, tmp)
}

fn create_test_buffer(storage: &Storage) -> i64 {
    let conn = storage.conn();
    let conn = conn.lock();
    conn.execute("INSERT INTO buffers (name, path) VALUES ('test', '/test')", [])
        .unwrap();
    conn.last_insert_rowid()
}

#[test]
fn test_insert_and_get_chunk() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = create_test_buffer(&storage);
    let chunk = NewChunk {
        buffer_id,
        file_path: "src/main.rs".to_string(),
        offset_start: 0,
        offset_end: 100,
        line_start: 1,
        line_end: 10,
        hash: vec![0x01, 0x02, 0x03],
        language: Some("rust".to_string()),
        chunk_type: Some("function".to_string()),
        token_count: Some(50),
    };
    let id = storage.insert_chunk(&chunk).unwrap();
    assert!(id > 0);
    let retrieved = storage.get_chunk(id).unwrap().unwrap();
    assert_eq!(retrieved.file_path, "src/main.rs");
    assert_eq!(retrieved.language, Some("rust".to_string()));
}

#[test]
fn test_chunk_content() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = create_test_buffer(&storage);
    let chunk = NewChunk {
        buffer_id,
        file_path: "src/main.rs".to_string(),
        offset_start: 0,
        offset_end: 100,
        line_start: 1,
        line_end: 10,
        hash: vec![0x01, 0x02, 0x03],
        language: None,
        chunk_type: None,
        token_count: None,
    };
    let id = storage.insert_chunk(&chunk).unwrap();
    storage.insert_chunk_content(id, "fn main() {}").unwrap();
    let content = storage.get_chunk_content(id).unwrap().unwrap();
    assert_eq!(content, "fn main() {}");
}

#[test]
fn test_list_chunks() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = create_test_buffer(&storage);
    for i in 0..3 {
        let chunk = NewChunk {
            buffer_id,
            file_path: format!("src/file{i}.rs"),
            offset_start: 0,
            offset_end: 100,
            line_start: 1,
            line_end: 10,
            hash: vec![i as u8],
            language: None,
            chunk_type: None,
            token_count: None,
        };
        storage.insert_chunk(&chunk).unwrap();
    }
    let chunks = storage.list_chunks(buffer_id).unwrap();
    assert_eq!(chunks.len(), 3);
}
