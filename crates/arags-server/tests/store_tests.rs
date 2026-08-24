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
