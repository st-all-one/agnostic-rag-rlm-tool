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

use arags_storage::Storage;
use arags_storage::sqlite::buffers::NewBuffer;
use rusqlite::Connection;
use tempfile::TempDir;

#[test]
fn test_storage_open() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    assert!(storage.path().exists());
}

#[test]
fn test_pooled_mode_opens() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open_pooled(tmp.path(), 4).unwrap();
    assert_eq!(
        storage.mode(),
        arags_storage::sqlite::conn::StorageMode::Pooled
    );
}

#[test]
fn test_pooled_concurrent_queries() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open_pooled(tmp.path(), 4).unwrap();

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let s = storage.clone();
            std::thread::spawn(move || {
                let conn = s.connection().unwrap();
                conn.execute(|c| {
                    c.prepare("SELECT 1").unwrap();
                    Ok(())
                })
                .unwrap();
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn test_ensure_fts5_available() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    storage.ensure_fts5_available().unwrap();
}

#[test]
fn test_backup_and_verify() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    storage.ensure_fts5_available().unwrap();
    storage.verify().unwrap();

    let id = storage
        .insert_buffer(&NewBuffer {
            name: "proj".into(),
            path: "/proj".into(),
        })
        .unwrap();
    assert!(id > 0);

    let backup_path = tmp.path().join("backup.db");
    storage.backup(&backup_path).unwrap();
    assert!(backup_path.exists());

    let conn = Connection::open(&backup_path).unwrap();
    let check: String = conn
        .query_row("PRAGMA integrity_check(1)", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM buffers", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
