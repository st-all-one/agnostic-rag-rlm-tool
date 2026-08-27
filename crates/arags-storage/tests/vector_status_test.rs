//! Tests for vector-derivation failure tracking (issue `agnostic-rlm-rs-50ed`).
//!
//! Verifies migration 022 adds the `vector_status` column to the dedicated
//! vector-space tables, and that the `pending_vector` marking/reporting
//! storage methods round-trip correctly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]

use arags_storage::Storage;
use arags_storage::sqlite::chunks::NewChunk;
use rusqlite::Connection;
use tempfile::TempDir;

fn setup_storage() -> (Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (storage, tmp)
}

fn create_test_buffer(storage: &Storage) -> i64 {
    let conn = storage.conn();
    let conn = conn.lock();
    conn.execute(
        "INSERT INTO buffers (name, path) VALUES ('test', '/test')",
        [],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// Returns the set of column names present on `table` via `PRAGMA table_info`.
fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
    rows.map(|r| r.unwrap()).collect()
}

#[test]
fn vector_status_column_exists() {
    let (storage, _tmp) = setup_storage();
    let conn = storage.conn();
    let conn = conn.lock();

    for table in ["rlm_nodes", "explorations", "qa_cache"] {
        let cols = columns(&conn, table);
        assert!(
            cols.contains(&"vector_status".to_string()),
            "{table} missing vector_status column; got {cols:?}"
        );
    }
}

#[test]
fn mark_and_query_pending_vector() {
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
    let id1 = storage.insert_chunk(&chunk).unwrap();
    let id2 = storage.insert_chunk(&chunk).unwrap();

    // Nothing pending initially.
    assert!(storage.chunks_pending_vector(buffer_id).unwrap().is_empty());

    // Mark one chunk pending.
    storage
        .mark_chunks_pending_vector(buffer_id, &[id1])
        .unwrap();

    let pending = storage.chunks_pending_vector(buffer_id).unwrap();
    assert_eq!(pending, vec![id1]);

    // The row's status reflects the marker.
    let status = {
        let conn = storage.conn();
        let conn = conn.lock();
        conn.query_row("SELECT status FROM chunks WHERE id = ?1", [id1], |r| {
            r.get::<_, String>(0)
        })
        .unwrap()
    };
    assert_eq!(status, "pending_vector");

    // id2 is unaffected.
    let status2 = {
        let conn = storage.conn();
        let conn = conn.lock();
        conn.query_row("SELECT status FROM chunks WHERE id = ?1", [id2], |r| {
            r.get::<_, String>(0)
        })
        .unwrap()
    };
    assert_eq!(status2, "active");

    // Marking an empty batch is a no-op.
    storage.mark_chunks_pending_vector(buffer_id, &[]).unwrap();
    assert_eq!(storage.chunks_pending_vector(buffer_id).unwrap().len(), 1);
}

#[test]
fn mark_and_query_rlm_nodes_pending_vector() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = create_test_buffer(&storage);

    let conn = storage.conn();
    let conn = conn.lock();
    conn.execute(
        "INSERT INTO rlm_nodes (node_id, buffer_id, project, level, subject, summary_text, created_at, updated_at, last_accessed_at) \
         VALUES ('n1', ?1, 'p', 1, 'f.rs', 's', 0, 0, 0), ('n2', ?1, 'p', 1, 'g.rs', 's', 0, 0, 0)",
        [buffer_id],
    )
    .unwrap();
    let id1 = conn
        .query_row("SELECT id FROM rlm_nodes WHERE node_id='n1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let id2: i64 = conn
        .query_row("SELECT id FROM rlm_nodes WHERE node_id='n2'", [], |r| {
            r.get(0)
        })
        .unwrap();
    drop(conn);

    assert!(
        storage
            .rlm_nodes_pending_vector(buffer_id)
            .unwrap()
            .is_empty()
    );

    storage
        .mark_rlm_nodes_pending_vector(buffer_id, &[id1])
        .unwrap();

    let pending = storage.rlm_nodes_pending_vector(buffer_id).unwrap();
    assert_eq!(pending, vec![id1]);

    let conn = storage.conn();
    let conn = conn.lock();
    let vs: String = conn
        .query_row(
            "SELECT vector_status FROM rlm_nodes WHERE id = ?1",
            [id1],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(vs, "pending_vector");

    let vs2: String = conn
        .query_row(
            "SELECT vector_status FROM rlm_nodes WHERE id = ?1",
            [id2],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(vs2, "indexed");
}

#[test]
fn mark_and_query_qa_cache_pending_vector() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = create_test_buffer(&storage);

    let conn = storage.conn();
    let conn = conn.lock();
    conn.execute(
        "INSERT INTO qa_cache (cache_id, buffer_id, project, question_text, question_hash, answer_text, created_at, last_accessed_at) \
         VALUES ('c1', ?1, 'p', 'q1', 'h1', 'a1', 0, 0), ('c2', ?1, 'p', 'q2', 'h2', 'a2', 0, 0)",
        [buffer_id],
    )
    .unwrap();
    let id1 = conn
        .query_row("SELECT id FROM qa_cache WHERE cache_id='c1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    drop(conn);

    assert!(storage.qa_cache_pending_vector("p").unwrap().is_empty());

    storage.mark_qa_cache_pending_vector(&[id1]).unwrap();

    let pending = storage.qa_cache_pending_vector("p").unwrap();
    assert_eq!(pending, vec![id1]);
}

#[test]
fn mark_and_query_explorations_pending_vector() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = create_test_buffer(&storage);

    let conn = storage.conn();
    let conn = conn.lock();
    conn.execute(
        "INSERT INTO explorations (exploration_id, project, buffer_id, goal, body, summary, created_by, template_version, created_at, updated_at, last_accessed_at) \
         VALUES ('e1', 'p', ?1, 'g', X'00', 's', 'u', 'v1', 0, 0, 0), ('e2', 'p', ?1, 'g2', X'00', 's2', 'u', 'v1', 0, 0, 0)",
        [buffer_id],
    )
    .unwrap();
    let id1 = conn
        .query_row(
            "SELECT id FROM explorations WHERE exploration_id='e1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);

    assert!(
        storage
            .explorations_pending_vector(buffer_id)
            .unwrap()
            .is_empty()
    );

    storage
        .mark_explorations_pending_vector(buffer_id, &[id1])
        .unwrap();

    let pending = storage.explorations_pending_vector(buffer_id).unwrap();
    assert_eq!(pending, vec![id1]);
}
