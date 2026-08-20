#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arlm_search::entity::EntitySearch;
use arlm_storage::sqlite::buffers::NewBuffer;
use arlm_storage::sqlite::chunks::NewChunk;
use arlm_storage::Storage;
use tempfile::TempDir;

fn setup() -> (EntitySearch, Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    let search = EntitySearch::new(storage.clone()).unwrap();
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

fn create_chunk_with_entities(
    storage: &Storage,
    buffer_id: i64,
    file_path: &str,
    content: &str,
) -> i64 {
    let chunk_id = storage
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
        .unwrap();

    storage.insert_chunk_content(chunk_id, content).unwrap();

    let entities = Storage::extract_entities(content, file_path);
    storage.insert_chunk_entities(chunk_id, &entities).unwrap();

    chunk_id
}

#[test]
fn test_extract_query_entities() {
    let entities = EntitySearch::extract_query_entities("validate_token in auth module");
    assert!(!entities.is_empty());
    assert!(entities.contains(&"validate_token".to_string()));
}

#[test]
fn test_entity_search_finds_match() {
    let (search, storage, _tmp) = setup();
    let buf = create_buffer(&storage, 0);

    let chunk_id = create_chunk_with_entities(
        &storage,
        buf,
        "src/auth.rs",
        "fn validate_token(token: &str) -> bool { true }",
    );

    let entities = vec!["validate_token".to_string()];
    let results = search.search(&entities, buf, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, chunk_id);
}

#[test]
fn test_entity_search_no_match() {
    let (search, storage, _tmp) = setup();
    let buf = create_buffer(&storage, 0);

    create_chunk_with_entities(
        &storage,
        buf,
        "src/main.rs",
        "fn main() { println!(\"hello\"); }",
    );

    let entities = vec!["validate_token".to_string()];
    let results = search.search(&entities, buf, 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_entity_search_buffer_filter() {
    let (search, storage, _tmp) = setup();
    let buf1 = create_buffer(&storage, 0);
    let buf2 = create_buffer(&storage, 1);

    create_chunk_with_entities(&storage, buf1, "a.rs", "fn alpha_bravo() {}");
    create_chunk_with_entities(&storage, buf2, "b.rs", "fn charlie_delta() {}");

    let entities = vec!["alpha_bravo".to_string()];
    let results = search.search(&entities, buf1, 10).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_entity_search_multiple_entities() {
    let (search, storage, _tmp) = setup();
    let buf = create_buffer(&storage, 0);

    let c1 = create_chunk_with_entities(&storage, buf, "auth.rs", "fn validate_token() {}");
    let c2 = create_chunk_with_entities(&storage, buf, "session.rs", "fn check_session() {}");

    let entities = vec!["validate_token".to_string(), "check_session".to_string()];
    let results = search.search(&entities, buf, 10).unwrap();
    assert_eq!(results.len(), 2);

    let chunk_ids: Vec<i64> = results.iter().map(|r| r.chunk_id).collect();
    assert!(chunk_ids.contains(&c1));
    assert!(chunk_ids.contains(&c2));
}

#[test]
fn test_entity_search_all() {
    let (search, storage, _tmp) = setup();
    let buf = create_buffer(&storage, 0);

    create_chunk_with_entities(&storage, buf, "a.rs", "fn alpha() {}");

    let entities = vec!["alpha".to_string()];
    let results = search.search_all(&entities, 10).unwrap();
    assert_eq!(results.len(), 1);
}
