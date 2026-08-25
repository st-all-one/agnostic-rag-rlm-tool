use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn test_purge_history_before_removes_only_old_rows() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(dir.path()).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let old = now - 10 * 86_400;

    // Seed one old and one current row by inserting then backdating.
    storage
        .insert_history(None, "old", Some("search"), None, None, None)
        .unwrap();
    storage
        .insert_history(None, "new", Some("search"), None, None, None)
        .unwrap();

    let conn = storage.conn();
    let guard = conn.lock();
    guard
        .execute(
            "UPDATE history SET created_at = ?1 WHERE query = 'old'",
            params![old],
        )
        .unwrap();
    drop(guard);

    let removed = storage.purge_history_before(now - 86_400).unwrap();
    assert_eq!(removed, 1);

    let remaining: Vec<HistoryEntry> = storage.get_history(None, 10).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].query, "new");
}
