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
use arlm_storage::sqlite::buffers::{Buffer, NewBuffer};
use tempfile::TempDir;

fn setup_storage() -> (Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (storage, tmp)
}

#[test]
fn test_insert_and_get_buffer() {
    let (storage, _tmp) = setup_storage();
    let buffer = NewBuffer {
        name: "my-project".to_string(),
        path: "/path/to/project".to_string(),
    };
    let id = storage.insert_buffer(&buffer).unwrap();
    assert!(id > 0);
    let retrieved: Buffer = storage.get_buffer(id).unwrap().unwrap();
    assert_eq!(retrieved.name, "my-project");
    assert_eq!(retrieved.path, "/path/to/project");
    assert!(retrieved.uuid.is_some());
}

#[test]
fn test_get_buffer_by_uuid() {
    let (storage, _tmp) = setup_storage();
    let buffer = NewBuffer {
        name: "my-project".to_string(),
        path: "/path/to/project".to_string(),
    };
    storage.insert_buffer(&buffer).unwrap();
    let buffers = storage.list_buffers().unwrap();
    let uuid = buffers[0].uuid.as_deref().unwrap();
    let retrieved = storage.get_buffer_by_uuid(uuid).unwrap().unwrap();
    assert_eq!(retrieved.name, "my-project");
}

#[test]
fn test_get_buffer_by_name() {
    let (storage, _tmp) = setup_storage();
    let buffer = NewBuffer {
        name: "my-project".to_string(),
        path: "/path/to/project".to_string(),
    };
    storage.insert_buffer(&buffer).unwrap();
    let retrieved = storage.get_buffer_by_name("my-project").unwrap().unwrap();
    assert_eq!(retrieved.path, "/path/to/project");
}

#[test]
fn test_list_buffers() {
    let (storage, _tmp) = setup_storage();
    for i in 0..3 {
        let buffer = NewBuffer {
            name: format!("project-{i}"),
            path: format!("/path/to/project-{i}"),
        };
        storage.insert_buffer(&buffer).unwrap();
    }
    let buffers = storage.list_buffers().unwrap();
    assert_eq!(buffers.len(), 3);
}

#[test]
fn test_update_buffer_counts() {
    let (storage, _tmp) = setup_storage();
    let buffer = NewBuffer {
        name: "my-project".to_string(),
        path: "/path/to/project".to_string(),
    };
    let id = storage.insert_buffer(&buffer).unwrap();
    storage.update_buffer_counts(id, 100, 10).unwrap();
    let retrieved = storage.get_buffer(id).unwrap().unwrap();
    assert_eq!(retrieved.total_chunks, 100);
    assert_eq!(retrieved.total_files, 10);
    assert!(retrieved.last_indexed_at.is_some());
}

#[test]
fn test_ensure_uuids_backfill() {
    let (storage, _tmp) = setup_storage();
    let buffer = NewBuffer {
        name: "project-a".to_string(),
        path: "/path/a".to_string(),
    };
    let id = storage.insert_buffer(&buffer).unwrap();
    {
        let conn = storage.conn();
        let conn = conn.lock();
        conn.execute("UPDATE buffers SET uuid = NULL WHERE id = ?1", [id])
            .unwrap();
    }
    assert!(storage.get_buffer(id).unwrap().unwrap().uuid.is_none());
    let count = storage.ensure_uuids().unwrap();
    assert_eq!(count, 1);
    assert!(storage.get_buffer(id).unwrap().unwrap().uuid.is_some());
}
