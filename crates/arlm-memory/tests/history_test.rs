#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use arlm_memory::history::*;
use arlm_storage::Storage;
use tempfile::TempDir;

fn setup() -> (HistoryManager, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (HistoryManager::new(storage), tmp)
}

#[test]
fn test_record_and_recent() {
    let (mgr, _tmp) = setup();

    let id = mgr
        .record(
            None,
            "find bugs",
            Some("search"),
            Some(5),
            Some(23),
            Some("opencode"),
        )
        .unwrap();
    assert!(id > 0);

    let recent = mgr.recent(None, 10).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].query, "find bugs");
    assert_eq!(recent[0].used_by, Some("opencode".to_string()));
}

#[test]
fn test_count() {
    let (mgr, _tmp) = setup();

    assert_eq!(mgr.count(None).unwrap(), 0);

    mgr.record(None, "q1", None, None, None, None).unwrap();
    mgr.record(None, "q2", None, None, None, None).unwrap();

    assert_eq!(mgr.count(None).unwrap(), 2);
}

#[test]
fn test_recent_limit() {
    let (mgr, _tmp) = setup();

    for i in 0..5 {
        mgr.record(None, &format!("query {i}"), None, None, None, None)
            .unwrap();
    }

    let recent = mgr.recent(None, 3).unwrap();
    assert_eq!(recent.len(), 3);
}
