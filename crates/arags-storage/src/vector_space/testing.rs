//! Tests for the generic [`VectorSpaceStore`] core shared by all dedicated
//! secondary vector spaces.

use super::*;
use std::time::Duration;

fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn sample(dims: usize, seed: f32) -> Vec<f32> {
    vec![seed; dims]
}

#[test]
fn insert_search_and_upsert_by_key() {
    let dir = temp_dir();
    let store = VectorSpaceStore::open(dir.path(), "t.usearch", 4, true).expect("open");
    store.insert(1, &sample(4, 0.1)).expect("insert");
    store.insert(2, &sample(4, 0.9)).expect("insert");

    let hits = store.search(&sample(4, 0.9), 2).expect("search");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, 2);
    assert!(hits[0].similarity > 0.999);

    // Upsert replaces by key.
    store.insert(2, &sample(4, 0.05)).expect("upsert");
    assert_eq!(store.len(), 2);
}

#[test]
fn dimension_mismatch_is_rejected() {
    let dir = temp_dir();
    let store = VectorSpaceStore::open(dir.path(), "t.usearch", 4, true).expect("open");
    assert!(store.insert(1, &sample(8, 0.5)).is_err());
}

#[test]
fn manual_persist_mode_never_writes_implicitly_and_flushes_on_demand() {
    let dir = temp_dir();
    let store = VectorSpaceStore::open(dir.path(), "t.usearch", 4, false).expect("open");
    store.insert(7, &sample(4, 0.5)).expect("insert");
    assert!(store.is_dirty(), "manual mode keeps mutations unsaved");

    // Reopening the same file must not see unflushed data.
    {
        let reopened = VectorSpaceStore::open(dir.path(), "t.usearch", 4, false).expect("reopen");
        assert_eq!(reopened.len(), 0);
    }

    store.persist().expect("flush");
    assert!(!store.is_dirty());
    let reopened = VectorSpaceStore::open(dir.path(), "t.usearch", 4, false).expect("reopen 2");
    assert_eq!(reopened.len(), 1);

    // Persisting again without mutations is a no-op.
    store.persist().expect("idempotent persist");
}

#[test]
fn auto_persist_debounces_bursts() {
    let dir = temp_dir();
    let store = VectorSpaceStore::open(dir.path(), "t.usearch", 4, true).expect("open");

    // First mutation saves immediately (lazy timer init).
    store.insert(1, &sample(4, 0.5)).expect("insert");
    assert!(!store.is_dirty(), "first mutation must be flushed");

    // Burst within the debounce window: still dirty until the window elapses.
    store.insert(2, &sample(4, 0.6)).expect("insert 2");
    store.delete(1).expect("delete");
    assert!(store.is_dirty(), "debounce keeps burst unsaved");

    // After the window elapses the next mutation flushes everything.
    std::thread::sleep(Duration::from_millis(SAVE_DEBOUNCE_MS + 50));
    store.insert(3, &sample(4, 0.7)).expect("insert 3");
    assert!(!store.is_dirty(), "post-window mutation triggers flush");
}
