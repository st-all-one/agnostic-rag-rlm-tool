#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::float_cmp
)]

use std::sync::atomic::{AtomicU64, Ordering};

use arlm_storage::Storage;
use arlm_storage::sqlite::buffers::NewBuffer;
use arlm_storage::sqlite::summaries::Summary;
use tempfile::TempDir;

static BUFFER_SEQ: AtomicU64 = AtomicU64::new(0);

fn setup_storage() -> (Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (storage, tmp)
}

fn make_buffer(storage: &Storage) -> i64 {
    let n = BUFFER_SEQ.fetch_add(1, Ordering::SeqCst);
    storage
        .insert_buffer(&NewBuffer {
            name: format!("proj-{n}"),
            path: format!("/proj-{n}"),
        })
        .unwrap()
}

#[test]
fn test_insert_and_get_summaries() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = make_buffer(&storage);
    let id = storage
        .insert_summary(
            buffer_id,
            "module summary",
            "module",
            Some(&[1, 2, 3]),
            Some("hash1"),
            0.9,
            Some(42),
            None,
        )
        .unwrap();
    assert!(id > 0);

    let summaries = storage.get_summaries(buffer_id).unwrap();
    assert_eq!(summaries.len(), 1);
    let s = &summaries[0];
    assert_eq!(s.scope, "module");
    assert_eq!(s.source_chunk_ids, Some(vec![1, 2, 3]));
    assert_eq!(s.source_hash.as_deref(), Some("hash1"));
}

#[test]
fn test_get_project_summary() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = make_buffer(&storage);
    storage
        .insert_summary(buffer_id, "file", "file", None, None, 0.5, None, None)
        .unwrap();
    storage
        .insert_summary(
            buffer_id,
            "project",
            "project",
            None,
            Some("ph"),
            1.0,
            None,
            None,
        )
        .unwrap();

    let project = storage.get_project_summary(buffer_id).unwrap();
    assert!(project.is_some());
    assert_eq!(project.unwrap().content, "project");
}

#[test]
fn test_get_summary_by_source_hash() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = make_buffer(&storage);
    storage
        .insert_summary(
            buffer_id,
            "content",
            "module",
            None,
            Some("abc"),
            0.8,
            None,
            None,
        )
        .unwrap();

    let found = storage
        .get_summary_by_source_hash(buffer_id, "abc")
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().content, "content");
}

#[test]
fn test_source_chunk_ids_roundtrip() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = make_buffer(&storage);
    storage
        .insert_summary(
            buffer_id,
            "c",
            "file",
            Some(&[10, 20]),
            None,
            0.1,
            Some(5),
            None,
        )
        .unwrap();
    let summaries: Vec<Summary> = storage.get_summaries(buffer_id).unwrap();
    assert_eq!(summaries[0].source_chunk_ids, Some(vec![10, 20]));
}

#[test]
fn test_search_summaries_by_buffer() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = make_buffer(&storage);
    storage
        .insert_summary(
            buffer_id,
            "authentication module handles tokens",
            "module",
            None,
            None,
            0.9,
            None,
            None,
        )
        .unwrap();
    storage
        .insert_summary(
            buffer_id,
            "unrelated content here",
            "file",
            None,
            None,
            0.9,
            None,
            None,
        )
        .unwrap();
    let other = make_buffer(&storage);
    storage
        .insert_summary(
            other,
            "authentication module in other buffer",
            "module",
            None,
            None,
            0.9,
            None,
            None,
        )
        .unwrap();

    let hits = storage
        .search_summaries("authentication", buffer_id, 10)
        .unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.buffer_id == buffer_id));
    assert_eq!(hits[0].scope, "module");
}

#[test]
fn test_search_summaries_all() {
    let (storage, _tmp) = setup_storage();
    let b1 = make_buffer(&storage);
    let b2 = make_buffer(&storage);
    storage
        .insert_summary(
            b1,
            "vector store indexing",
            "module",
            None,
            None,
            0.9,
            None,
            None,
        )
        .unwrap();
    storage
        .insert_summary(
            b2,
            "vector store retrieval",
            "project",
            None,
            None,
            0.9,
            None,
            None,
        )
        .unwrap();

    let hits = storage.search_summaries_all("vector", 10).unwrap();
    assert_eq!(hits.len(), 2);
}

#[test]
fn test_get_summary_by_id() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = make_buffer(&storage);
    let id = storage
        .insert_summary(
            buffer_id,
            "by id lookup",
            "file",
            None,
            None,
            0.9,
            None,
            None,
        )
        .unwrap();

    let s = storage.get_summary(id).unwrap();
    assert!(s.is_some());
    assert_eq!(s.unwrap().content, "by id lookup");
}
