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

use arlm_storage::Storage;
use rusqlite::params;
use tempfile::TempDir;

fn setup_storage() -> (Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (storage, tmp)
}

fn create_test_task(storage: &Storage) -> i64 {
    let conn = storage.conn();
    let conn = conn.lock();
    conn.execute(
        "INSERT INTO buffers (name, path) VALUES ('test', '/test')",
        [],
    )
    .unwrap();
    let buffer_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO tasks (buffer_id) VALUES (?1)",
        params![buffer_id],
    )
    .unwrap();
    conn.last_insert_rowid()
}

#[test]
fn test_insert_finding() {
    let (storage, _tmp) = setup_storage();
    let task_id = create_test_task(&storage);
    let finding_id = storage
        .insert_finding(task_id, None, Some("bug"), "Found a bug", Some(0.9))
        .unwrap();
    assert!(finding_id > 0);
    let findings = storage.get_findings_for_task(task_id).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].finding_type, Some("bug".to_string()));
}
