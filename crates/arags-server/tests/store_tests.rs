#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::pedantic
)]

//! Integration tests for the typed store layer (`arags_server::store`).
//!
//! Storage is opened in single-connection mode (SQLite in a temp dir) so the
//! tests are fast and hermetic; the same functions are used by the server's
//! pooled mode.

use std::collections::HashMap;

use anyhow::Context;
use arags_server::store;
use arags_storage::Storage;
use tempfile::TempDir;

fn setup() -> (Storage, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    // Single-header DB at a fixed path.
    let storage = Storage::open(dir.path()).expect("open storage");
    (storage, dir)
}

// ── Projects ───────────────────────────────────────────────────────────────

#[test]
fn test_insert_and_get_project_by_name() {
    let (storage, _dir) = setup();
    let id = store::insert_project(&storage, "my-project", "/path/to/project").unwrap();
    assert!(id > 0);

    let row = store::get_project_by_name(&storage, "my-project")
        .unwrap()
        .expect("project exists");
    assert_eq!(row.id, id);
    assert_eq!(row.path, "/path/to/project");
    assert!(row.uuid.is_some());
}

#[test]
fn test_get_project_by_uuid() {
    let (storage, _dir) = setup();
    store::insert_project(&storage, "p1", "/p1").unwrap();

    let by_name = store::get_project_by_name(&storage, "p1").unwrap().unwrap();
    let by_uuid = store::get_project_by_uuid(&storage, by_name.uuid.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(by_uuid.id, by_name.id);
}

#[test]
fn test_list_projects() {
    let (storage, _dir) = setup();
    for i in 0..3 {
        store::insert_project(&storage, &format!("proj-{i}"), &format!("/p{i}")).unwrap();
    }
    let projects = store::list_projects(&storage).unwrap();
    assert_eq!(projects.len(), 3);
}

// ── Re-index idempotency (stopgap for agnostic-rlm-rs-20cd) ──────────────────

#[test]
fn test_reindex_does_not_duplicate_chunks() {
    let (storage, _dir) = setup();

    // Replicates the handler's Phase 0 behaviour: delete the buffer's chunks
    // (cascade) and re-insert the same content. Counts must stay stable.
    let buffer_id = store::insert_project(&storage, "reindex-test", "/tmp/reindex-test").unwrap();
    let mk = |fp: &str| arags_server::indexing::IndexedChunk {
        file_path: fp.to_string(),
        line_start: 1,
        line_end: 3,
        content: "fn main() {}".to_string(),
        hash: format!("{fp}-hash"),
        language: Some("rust".to_string()),
        chunk_type: "code".to_string(),
    };
    let chunks = [mk("a.rs"), mk("b.rs")];
    let flat: Vec<(&str, &arags_server::indexing::IndexedChunk)> =
        chunks.iter().map(|c| (c.file_path.as_str(), c)).collect();

    store::insert_chunks_batched(&storage, buffer_id, &flat, 100, &HashMap::new(), None, None)
        .unwrap();
    assert_eq!(
        storage.count_chunks(buffer_id).unwrap(),
        2,
        "two chunks after first insert"
    );

    // Simulate a repeated index: purge the buffer, then re-insert identical chunks.
    let (ids, deleted_files) = store::delete_chunks_for_buffer(&storage, buffer_id).unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(deleted_files, 2);
    assert_eq!(
        storage.count_chunks(buffer_id).unwrap(),
        0,
        "buffer emptied by cascade delete"
    );

    store::insert_chunks_batched(&storage, buffer_id, &flat, 100, &HashMap::new(), None, None)
        .unwrap();
    assert_eq!(
        storage.count_chunks(buffer_id).unwrap(),
        2,
        "re-index must not duplicate chunks"
    );

    // FTS and entity rows must also be gone, or the next search would see stale hits.
    let fts_rows: i64 = storage
        .connection()
        .unwrap()
        .execute(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE rowid IN (SELECT id FROM chunks WHERE buffer_id = ?1)",
                rusqlite::params![buffer_id],
                |r| r.get(0),
            )
            .context("count chunks_fts for buffer")
        })
        .unwrap();
    assert_eq!(fts_rows, 2, "FTS rows present for the re-inserted chunks");
}

// ── Authorship propagation (issue `agnostic-rlm-rs-786a`) ─────────────────────

#[test]
fn insert_chunks_batched_populates_created_by_and_model() {
    let (storage, _dir) = setup();
    let buffer_id = store::insert_project(&storage, "auth-test", "/tmp/auth-test").unwrap();

    let mk = |fp: &str| arags_server::indexing::IndexedChunk {
        file_path: fp.to_string(),
        line_start: 1,
        line_end: 3,
        content: "fn main() {}".to_string(),
        hash: format!("{fp}-hash"),
        language: Some("rust".to_string()),
        chunk_type: "code".to_string(),
    };
    let chunks = [mk("a.rs"), mk("b.rs")];
    let flat: Vec<(&str, &arags_server::indexing::IndexedChunk)> =
        chunks.iter().map(|c| (c.file_path.as_str(), c)).collect();

    let created_by = Some("alice");
    let model = Some("sentence-minilm");
    store::insert_chunks_batched(
        &storage,
        buffer_id,
        &flat,
        100,
        &HashMap::new(),
        created_by,
        model,
    )
    .unwrap();

    let rows: Vec<(Option<String>, Option<String>, i64)> = storage
        .connection()
        .unwrap()
        .execute(|conn| {
            let mut stmt = conn
                .prepare("SELECT created_by, model, is_active FROM chunks WHERE buffer_id = ?1")?;
            let rows = stmt
                .query_map(rusqlite::params![buffer_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .filter_map(std::result::Result::ok)
                .collect();
            Ok(rows)
        })
        .unwrap();

    assert_eq!(rows.len(), 2, "two active chunks persisted");
    for (cb, m, active) in rows {
        assert_eq!(active, 1, "chunk is active");
        assert_eq!(cb.as_deref(), Some("alice"), "created_by populated");
        assert_eq!(m.as_deref(), Some("sentence-minilm"), "model populated");
    }
}
