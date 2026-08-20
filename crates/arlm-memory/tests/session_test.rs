#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::all, clippy::pedantic, clippy::nursery)]

use arlm_memory::session::*;
use arlm_storage::Storage;
use tempfile::TempDir;

fn setup() -> (SessionManager, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (SessionManager::new(storage).unwrap(), tmp)
}

#[test]
fn test_create_session() {
    let (mgr, _tmp) = setup();
    let id = mgr.create("my-project", "Auth Analysis").unwrap();
    assert!(id.starts_with("s_"));

    let session = mgr.get(&id).unwrap().unwrap();
    assert_eq!(session.project_name, "my-project");
    assert_eq!(session.title, "Auth Analysis");
}

#[test]
fn test_add_and_get_context() {
    let (mgr, _tmp) = setup();
    let id = mgr.create("proj", "title").unwrap();

    let v1 = mgr.add_context(&id, "context 0").unwrap();
    assert_eq!(v1, 1);

    let v2 = mgr.add_context(&id, "context 1").unwrap();
    assert_eq!(v2, 2);

    let latest = mgr.get_latest_context(&id).unwrap().unwrap();
    assert_eq!(latest.version, 2);
    assert_eq!(latest.payload, "context 1");

    let all = mgr.get_contexts(&id).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn test_session_history() {
    let (mgr, _tmp) = setup();
    let id = mgr.create("proj", "title").unwrap();

    mgr.record_query(&id, "what is auth?", Some("Auth is..."))
        .unwrap();
    mgr.record_query(&id, "how does login work?", None).unwrap();

    let history = mgr.get_history(&id, 10).unwrap();
    assert_eq!(history.len(), 2);
    let queries: Vec<&str> = history.iter().map(|(q, _, _)| q.as_str()).collect();
    assert!(queries.contains(&"what is auth?"));
    assert!(queries.contains(&"how does login work?"));
}

#[test]
fn test_get_nonexistent_session() {
    let (mgr, _tmp) = setup();
    let result = mgr.get("s_nonexistent").unwrap();
    assert!(result.is_none());
}
