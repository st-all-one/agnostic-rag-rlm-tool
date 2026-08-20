#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use arlm_memory::project::*;
use arlm_storage::Storage;
use tempfile::TempDir;

fn setup() -> (ProjectManager, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (ProjectManager::new(storage), tmp)
}

#[test]
fn test_create_project() {
    let (mgr, _tmp) = setup();
    let info = mgr
        .create(&CreateProjectOptions {
            name: "my-proj".to_string(),
            path: std::path::PathBuf::from("/tmp/my-proj"),
        })
        .unwrap();

    assert_eq!(info.name, "my-proj");
    assert_eq!(info.path, std::path::PathBuf::from("/tmp/my-proj"));
    assert!(info.id > 0);
}

#[test]
fn test_create_duplicate_project_fails() {
    let (mgr, _tmp) = setup();
    let opts = CreateProjectOptions {
        name: "dup".to_string(),
        path: std::path::PathBuf::from("/tmp/dup"),
    };
    mgr.create(&opts).unwrap();
    let result = mgr.create(&opts);
    assert!(result.is_err());
}

#[test]
fn test_list_projects() {
    let (mgr, _tmp) = setup();
    mgr.create(&CreateProjectOptions {
        name: "a".to_string(),
        path: std::path::PathBuf::from("/a"),
    })
    .unwrap();
    mgr.create(&CreateProjectOptions {
        name: "b".to_string(),
        path: std::path::PathBuf::from("/b"),
    })
    .unwrap();

    let list = mgr.list().unwrap();
    assert_eq!(list.len(), 2);
}

#[test]
fn test_get_project() {
    let (mgr, _tmp) = setup();
    mgr.create(&CreateProjectOptions {
        name: "findme".to_string(),
        path: std::path::PathBuf::from("/findme"),
    })
    .unwrap();

    let found = mgr.get("findme").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "findme");

    let missing = mgr.get("nope").unwrap();
    assert!(missing.is_none());
}

#[test]
fn test_forget_project() {
    let (mgr, _tmp) = setup();
    mgr.create(&CreateProjectOptions {
        name: "goner".to_string(),
        path: std::path::PathBuf::from("/goner"),
    })
    .unwrap();

    mgr.forget("goner").unwrap();
    let found = mgr.get("goner").unwrap();
    assert!(found.is_none());
}
