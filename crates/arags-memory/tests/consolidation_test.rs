#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use std::sync::Arc;

use arags_memory::consolidation::*;
use arags_storage::sqlite::buffers::NewBuffer;
use arags_storage::sqlite::chunks::NewChunk;
use arags_storage::{Storage, VectorEntry, VectorStore};
use tempfile::TempDir;

fn setup() -> (ConsolidationEngine, Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    let engine = ConsolidationEngine::new(storage.clone());
    (engine, storage, tmp)
}

fn create_buffer(storage: &Storage) -> i64 {
    storage
        .insert_buffer(&NewBuffer {
            name: "test".to_string(),
            path: "/test".to_string(),
        })
        .unwrap()
}

#[test]
fn test_remove_duplicates() {
    let (engine, storage, _tmp) = setup();
    let buffer_id = create_buffer(&storage);

    let hash = vec![1, 2, 3];
    for _ in 0..3 {
        storage
            .insert_chunk(&NewChunk {
                buffer_id,
                file_path: "src/main.rs".to_string(),
                offset_start: 0,
                offset_end: 100,
                line_start: 1,
                line_end: 10,
                hash: hash.clone(),
                language: None,
                chunk_type: None,
                token_count: None,
            })
            .unwrap();
    }

    let opts = ConsolidateOptions {
        deduplicate: true,
        min_pattern_confidence: 0.0,
        dry_run: false,
    };

    let result = engine.consolidate(buffer_id, &opts).unwrap();
    assert_eq!(result.duplicate_chunks_removed, 2);

    let remaining = storage.count_chunks(buffer_id).unwrap();
    assert_eq!(remaining, 1);
}

#[test]
fn test_remove_low_confidence_patterns() {
    let (engine, storage, _tmp) = setup();
    let buffer_id = create_buffer(&storage);

    storage
        .insert_pattern(Some(buffer_id), None, "high", None, None, Some(0.9))
        .unwrap();
    storage
        .insert_pattern(Some(buffer_id), None, "low", None, None, Some(0.1))
        .unwrap();

    let opts = ConsolidateOptions {
        deduplicate: false,
        min_pattern_confidence: 0.5,
        dry_run: false,
    };

    let result = engine.consolidate(buffer_id, &opts).unwrap();
    assert_eq!(result.low_confidence_patterns_removed, 1);
}

#[test]
fn test_remove_duplicates_purges_child_rows_without_fk_violation() {
    let (engine, storage, _tmp) = setup();
    let buffer_id = create_buffer(&storage);

    // Three chunks that share the same content hash (duplicates).
    let hash = vec![9, 9, 9];
    let mut ids = Vec::new();
    for _ in 0..3 {
        let id = storage
            .insert_chunk(&NewChunk {
                buffer_id,
                file_path: "src/lib.rs".to_string(),
                offset_start: 0,
                offset_end: 100,
                line_start: 1,
                line_end: 10,
                hash: hash.clone(),
                language: None,
                chunk_type: None,
                token_count: None,
            })
            .unwrap();
        ids.push(id);
    }

    // Child rows referencing chunks(id) via FKs without ON DELETE CASCADE:
    // chunk_texts, a task, and a finding linked to that task. The raw
    // connection guard must be dropped before calling Storage methods that
    // lock the same connection internally (the mutex is not reentrant).
    {
        let guard = storage.conn();
        let conn = guard.lock();
        for id in &ids {
            conn.execute(
                "INSERT INTO chunk_texts(chunk_id, content) VALUES (?1, 'x')",
                rusqlite::params![id],
            )
            .unwrap();
        }
    }
    storage
        .insert_task(buffer_id, Some(ids[0]), Some("p"))
        .unwrap();
    let task_id = {
        let guard = storage.conn();
        let conn = guard.lock();
        conn.query_row(
            "SELECT id FROM tasks WHERE chunk_id = ?1",
            rusqlite::params![ids[0]],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    storage
        .insert_finding(task_id, Some(ids[0]), Some("t"), "f", Some(0.5))
        .unwrap();

    let opts = ConsolidateOptions {
        deduplicate: true,
        min_pattern_confidence: 0.0,
        dry_run: false,
    };

    // Must not fail with a foreign-key constraint violation.
    let result = engine.consolidate(buffer_id, &opts).unwrap();
    assert_eq!(result.duplicate_chunks_removed, 2);

    let remaining = storage.count_chunks(buffer_id).unwrap();
    assert_eq!(remaining, 1);

    // Dependent rows tied to the two deleted chunks are gone; the kept
    // chunk keeps its chunk_texts row, and its task/finding are removed.
    let conn = storage.conn();
    let conn = conn.lock();
    let text_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM chunk_texts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(text_rows, 1);
    let task_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(task_rows, 0);
    let finding_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM findings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(finding_rows, 0);
}

#[test]
fn test_consolidate_empty_project() {
    let (engine, storage, _tmp) = setup();
    let buffer_id = create_buffer(&storage);

    let opts = ConsolidateOptions::default();
    let result = engine.consolidate(buffer_id, &opts).unwrap();
    assert_eq!(result.duplicate_chunks_removed, 0);
    assert_eq!(result.low_confidence_patterns_removed, 0);
}

/// Deduplication must also purge the vectors of the removed chunks (issue
/// `agnostic-rag-rlm-tool-fa25`) so the usearch chunk count stays equal to the
/// SQLite chunk count. Without this, the orphan vectors cause a bootstrap
/// count-divergence that forces a full re-embed on every restart.
#[tokio::test]
async fn test_consolidate_purges_orphan_vectors_for_removed_chunks() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    let dims = 384usize;
    let vs = Arc::new(VectorStore::open_with_dims(tmp.path(), dims).await.unwrap());
    let engine = ConsolidationEngine::new(storage.clone()).with_vector_store(vs.clone());
    let buffer_id = create_buffer(&storage);

    let hash = vec![7u8, 7u8, 7u8];
    let mut ids = Vec::new();
    for _ in 0..3 {
        let id = storage
            .insert_chunk(&NewChunk {
                buffer_id,
                file_path: "src/main.rs".to_string(),
                offset_start: 0,
                offset_end: 100,
                line_start: 1,
                line_end: 10,
                hash: hash.clone(),
                language: None,
                chunk_type: None,
                token_count: None,
            })
            .unwrap();
        ids.push(id);
    }

    // Seed a vector for every chunk so the space mirrors SQLite (in sync).
    let entries: Vec<VectorEntry> = ids
        .iter()
        .map(|id| VectorEntry {
            chunk_id: u64::try_from(*id).unwrap(),
            buffer_id: u64::try_from(buffer_id).unwrap(),
            vector: vec![0.0f32; dims],
        })
        .collect();
    vs.insert_vectors(&entries).await.unwrap();
    assert_eq!(vs.count().await, 3);

    // Consolidate: 2 of the 3 duplicate chunks are removed.
    let opts = ConsolidateOptions {
        deduplicate: true,
        min_pattern_confidence: 0.0,
        dry_run: false,
    };
    let result = engine.consolidate(buffer_id, &opts).unwrap();
    assert_eq!(result.duplicate_chunks_removed, 2);

    // Only the kept chunk's vector should remain; the two consolidated-away
    // chunks must have had their vectors purged (no divergence for bootstrap).
    assert_eq!(vs.count().await, 1);
}
