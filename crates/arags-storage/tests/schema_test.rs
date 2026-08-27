//! Tests for migration 021 (temporal / versioning metadata).
//!
//! Opens a fresh database (which runs every migration, including 021) and
//! verifies the new columns and partial indices exist, that a chunk inserted
//! through the storage API gets the correct temporal defaults, and that
//! re-running migrations is idempotent.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_sign_loss
)]

use arags_storage::sqlite::Storage;
use arags_storage::sqlite::buffers::NewBuffer;
use arags_storage::sqlite::chunks::NewChunk;
use arags_storage::sqlite::schema::{MIGRATION_COUNT, run_migrations};
use rusqlite::Connection;

fn temp_storage() -> Storage {
    let dir = tempfile::tempdir().unwrap();
    Storage::open(dir.path()).unwrap()
}

/// Returns the set of column names present on `table` via `PRAGMA table_info`.
fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
    rows.map(|r| r.unwrap()).collect()
}

fn index_exists(conn: &Connection, table: &str, index: &str) -> bool {
    let mut stmt = conn
        .prepare(&format!("PRAGMA index_list({table})"))
        .unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
    rows.map(|r| r.unwrap()).any(|name| name == index)
}

#[test]
fn migration_021_columns_present_on_all_tables() {
    let storage = temp_storage();
    let conn = storage.conn();
    let guard = conn.lock();

    let chunk_cols = columns(&guard, "chunks");
    for col in [
        "version",
        "is_active",
        "superseded_by",
        "epoch",
        "created_by",
        "model",
    ] {
        assert!(
            chunk_cols.contains(&col.to_string()),
            "chunks missing {col}"
        );
    }

    // qa_cache already had `model` in 016 — skip it.
    let qa_cols = columns(&guard, "qa_cache");
    for col in [
        "version",
        "is_active",
        "superseded_by",
        "epoch",
        "created_by",
    ] {
        assert!(qa_cols.contains(&col.to_string()), "qa_cache missing {col}");
    }
    assert!(
        qa_cols.contains(&"model".to_string()),
        "qa_cache lost model"
    );

    // rlm_nodes already had `model` in 018 — skip it.
    let rlm_cols = columns(&guard, "rlm_nodes");
    for col in [
        "version",
        "is_active",
        "superseded_by",
        "epoch",
        "created_by",
    ] {
        assert!(
            rlm_cols.contains(&col.to_string()),
            "rlm_nodes missing {col}"
        );
    }
    assert!(
        rlm_cols.contains(&"model".to_string()),
        "rlm_nodes lost model"
    );

    // explorations already had created_by, model, epoch_created in 019 — skip.
    let exp_cols = columns(&guard, "explorations");
    for col in ["version", "is_active", "superseded_by"] {
        assert!(
            exp_cols.contains(&col.to_string()),
            "explorations missing {col}"
        );
    }
    assert!(
        exp_cols.contains(&"created_by".to_string()),
        "explorations lost created_by"
    );
    assert!(
        exp_cols.contains(&"model".to_string()),
        "explorations lost model"
    );
}

#[test]
fn migration_021_partial_indices_present() {
    let storage = temp_storage();
    let conn = storage.conn();
    let guard = conn.lock();

    assert!(index_exists(&guard, "chunks", "idx_chunks_active"));
    assert!(index_exists(&guard, "qa_cache", "idx_qa_cache_active"));
    assert!(index_exists(&guard, "rlm_nodes", "idx_rlm_nodes_active"));
    assert!(index_exists(
        &guard,
        "explorations",
        "idx_explorations_active"
    ));
}

#[test]
fn migration_021_chunk_defaults() {
    let storage = temp_storage();

    let buffer_id = storage
        .insert_buffer(&NewBuffer {
            name: "p".into(),
            path: "/p".into(),
        })
        .unwrap();

    let chunk = NewChunk {
        buffer_id,
        file_path: "src/main.rs".into(),
        offset_start: 0,
        offset_end: 10,
        line_start: 1,
        line_end: 2,
        hash: b"deadbeef".to_vec(),
        language: Some("rust".into()),
        chunk_type: Some("code".into()),
        token_count: Some(5),
    };
    let id = storage.insert_chunk(&chunk).unwrap();

    let conn = storage.conn();
    let guard = conn.lock();
    let (is_active, version, epoch, superseded_by): (i64, i64, i64, Option<i64>) = guard
        .query_row(
            "SELECT is_active, version, epoch, superseded_by FROM chunks WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();

    assert_eq!(is_active, 1);
    assert_eq!(version, 1);
    assert_eq!(epoch, 0);
    assert!(superseded_by.is_none());
}

#[test]
fn migration_021_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("store.db");

    // First open applies migrations up to 021.
    {
        let s = Storage::open(&db_path).unwrap();
        drop(s);
    }
    let before = MIGRATION_COUNT;

    // Re-open on the same file: run_migrations must be a no-op and succeed.
    {
        let s = Storage::open(&db_path).unwrap();
        let c = s.conn();
        let conn = c.lock();
        run_migrations(&conn).unwrap();
    }

    // And a brand-new DB must apply exactly MIGRATION_COUNT migrations.
    let storage = temp_storage();
    let conn = storage.conn();
    let guard = conn.lock();
    let applied: i64 = guard
        .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(applied as usize, before);
}
