use super::*;

fn temp_store(dims: usize) -> (tempfile::TempDir, RlmVectorStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RlmVectorStore::open(dir.path(), dims).expect("open");
    (dir, store)
}

#[test]
fn insert_search_and_delete_roundtrip() {
    let (_dir, store) = temp_store(4);
    assert!(store.is_empty());
    assert_eq!(store.dimensions(), 4);

    store.insert(10, &[1.0, 0.0, 0.0, 0.0]).expect("insert 10");
    store.insert(20, &[0.0, 1.0, 0.0, 0.0]).expect("insert 20");
    assert_eq!(store.len(), 2);

    let hits = store.search(&[1.0, 0.0, 0.0, 0.0], 2).expect("search");
    assert_eq!(hits[0].id, 10);
    assert!(hits[0].similarity > 0.99);

    // Replace keeps one entry per key.
    store.insert(10, &[0.0, 0.0, 1.0, 0.0]).expect("replace");
    assert_eq!(store.len(), 2);
    let hits = store.search(&[0.0, 0.0, 1.0, 0.0], 2).expect("search");
    assert_eq!(hits[0].id, 10);

    store.delete(10).expect("delete");
    assert_eq!(store.len(), 1);
    let hits = store.search(&[0.0, 0.0, 1.0, 0.0], 2).expect("search");
    assert_eq!(hits[0].id, 20);
}

#[test]
fn wrong_dimensionality_is_rejected() {
    let (_dir, store) = temp_store(4);
    let err = store.insert(1, &[1.0, 0.0]).expect_err("dim mismatch");
    assert!(err.to_string().contains("dimension mismatch"));
}

#[test]
fn persistence_across_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = RlmVectorStore::open(dir.path(), 2).expect("open");
        store.insert(7, &[0.5, 0.5]).expect("insert");
    }
    let reopened = RlmVectorStore::open(dir.path(), 2).expect("reopen");
    assert_eq!(reopened.len(), 1);
    let hits = reopened.search(&[0.5, 0.5], 1).expect("search");
    assert_eq!(hits[0].id, 7);
}
