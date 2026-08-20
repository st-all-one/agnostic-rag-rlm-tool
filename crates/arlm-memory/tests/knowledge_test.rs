#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use arlm_memory::knowledge::*;
use arlm_storage::Storage;
use arlm_storage::sqlite::buffers::NewBuffer;
use std::path::Path;
use tempfile::TempDir;

fn setup() -> (KnowledgeEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (KnowledgeEngine::new(storage), tmp)
}

fn create_project(engine: &KnowledgeEngine, name: &str) -> i64 {
    engine
        .storage()
        .insert_buffer(&NewBuffer {
            name: name.to_string(),
            path: "/tmp/test".to_string(),
        })
        .unwrap()
}

#[test]
fn test_index_file() {
    let (engine, tmp) = setup();
    let buffer_id = create_project(&engine, "test");

    let file_path = tmp.path().join("test.rs");
    std::fs::write(&file_path, "fn main() {\n    println!(\"hello\");\n}").unwrap();

    let opts = IndexOptions::default();
    let ids = engine.index_file(buffer_id, &file_path, &opts).unwrap();
    assert!(!ids.is_empty());

    let content = engine.get_chunk_content(ids[0]).unwrap().unwrap();
    assert!(content.contains("fn main"));
}

#[test]
fn test_index_directory() {
    let (engine, tmp) = setup();
    create_project(&engine, "test");

    let src_dir = tmp.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let file1 = src_dir.join("a.rs");
    let file2 = src_dir.join("b.py");
    std::fs::write(&file1, "fn main() {}").unwrap();
    std::fs::write(&file2, "print('hello')").unwrap();

    let opts = IndexOptions::default();
    let result = engine.index_directory("test", &src_dir, &opts).unwrap();

    assert_eq!(result.files_processed, 2);
    assert!(result.chunks_created >= 2);
}

#[test]
fn test_detect_language() {
    assert_eq!(
        detect_language(Path::new("main.rs")),
        Some("rust".to_string())
    );
    assert_eq!(
        detect_language(Path::new("app.py")),
        Some("python".to_string())
    );
    assert_eq!(
        detect_language(Path::new("index.html")),
        Some("html".to_string())
    );
    assert_eq!(detect_language(Path::new("unknown")), None);
}

#[test]
fn test_compute_hash() {
    let h1 = compute_hash(b"hello");
    let h2 = compute_hash(b"hello");
    let h3 = compute_hash(b"world");
    assert_eq!(h1, h2);
    assert_ne!(h1, h3);
}

#[test]
fn test_estimate_tokens() {
    assert!(estimate_tokens("hello world") > 0);
    assert!(estimate_tokens("") >= 1);
}
