#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss, clippy::cast_possible_wrap, clippy::cast_lossless, clippy::float_cmp)]

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
fn test_insert_and_get_task() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = create_test_buffer(&storage);
    let task_id = storage.insert_task(buffer_id, None, Some("{}")).unwrap();
    assert!(task_id > 0);
    let tasks = storage.get_pending_tasks(buffer_id, 10).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, task_id);
}

#[test]
fn test_complete_task() {
    let (storage, _tmp) = setup_storage();
    let buffer_id = create_test_buffer(&storage);
    let task_id = storage.insert_task(buffer_id, None, None).unwrap();
    storage.complete_task(task_id, "done").unwrap();
    let tasks = storage.get_pending_tasks(buffer_id, 10).unwrap();
    assert_eq!(tasks.len(), 0);
}
