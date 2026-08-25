//! Behavioral tests for the exploration vector space (plan 022): insert /
//! replace / delete / search semantics plus non-interference with the chunk
//! and question vector spaces (three dedicated `usearch` files).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arags_storage::Storage;
use arags_storage::exploration_vectors::ExplorationVectorStore;
use arags_storage::lance::vectors::{VectorEntry, VectorStore};
use arags_storage::qa_vectors::QuestionVectorStore;

fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn sample(dims: usize, seed: f32) -> Vec<f32> {
    vec![seed; dims]
}

#[test]
fn test_insert_search_and_replace_by_rowid() {
    let dir = temp_dir();
    let store = ExplorationVectorStore::open(dir.path(), 8).unwrap();

    // Map 1 points "east", map 2 points "west" (orthogonal-ish in 8d).
    let mut v1 = vec![0.0_f32; 8];
    v1[0] = 1.0;
    let mut v2 = vec![0.0_f32; 8];
    v2[1] = 1.0;
    store.insert(1, &v1).unwrap();
    store.insert(2, &v2).unwrap();
    assert_eq!(store.len(), 2);

    let hits = store.search(&v1, 2).unwrap();
    assert_eq!(hits[0].id, 1);
    assert!(hits[0].similarity > 0.999);

    // Upsert same key: no duplicate, new direction wins.
    let mut v1b = vec![0.0_f32; 8];
    v1b[7] = 1.0;
    store.insert(1, &v1b).unwrap();
    assert_eq!(store.len(), 2);
    let hits = store.search(&v1b, 5).unwrap();
    assert_eq!(hits[0].id, 1);

    store.delete(1).unwrap();
    assert_eq!(store.len(), 1);
    assert!(
        store.search(&v1b, 5).unwrap().iter().all(|h| h.id != 1),
        "deleted key must not resurface"
    );
}

#[test]
fn test_dimension_mismatch_is_rejected() {
    let dir = temp_dir();
    let store = ExplorationVectorStore::open(dir.path(), 8).unwrap();
    let err = store.insert(1, &sample(4, 0.1)).unwrap_err();
    assert!(err.to_string().contains("dimension mismatch"));
}

#[test]
fn test_index_persists_across_reopen() {
    let dir = temp_dir();
    {
        let store = ExplorationVectorStore::open(dir.path(), 8).unwrap();
        store.insert(7, &sample(8, 0.5)).unwrap();
    }
    let reopened = ExplorationVectorStore::open(dir.path(), 8).unwrap();
    assert_eq!(reopened.len(), 1);
    let hits = reopened.search(&sample(8, 0.5), 1).unwrap();
    assert_eq!(hits[0].id, 7);
}

#[tokio::test]
async fn test_three_vector_spaces_do_not_interfere() {
    let dir = temp_dir();
    let storage = Storage::open(dir.path()).unwrap();

    let chunks = VectorStore::open_with_dims(dir.path(), 8)
        .await
        .expect("chunk space");
    let questions = QuestionVectorStore::open_for_storage(&storage, 8).expect("question space");
    let explorations = ExplorationVectorStore::open_for_storage(&storage, 8).expect("map space");

    let east = {
        let mut v = vec![0.0_f32; 8];
        v[0] = 1.0;
        v
    };
    let west = {
        let mut v = vec![0.0_f32; 8];
        v[3] = 1.0;
        v
    };

    chunks
        .insert_vectors(&[VectorEntry {
            chunk_id: 1000,
            buffer_id: 1,
            vector: west.clone(),
        }])
        .await
        .expect("chunk insert");
    questions.insert(10, &west).unwrap();
    explorations.insert(20, &east).unwrap();

    // Each space only sees its own keys.
    assert!(
        explorations
            .search(&east, 10)
            .unwrap()
            .iter()
            .all(|h| h.id == 20)
    );
    assert!(
        questions
            .search(&west, 10)
            .unwrap()
            .iter()
            .all(|h| h.id == 10)
    );
    let chunk_hits = chunks
        .search_similar(&west, Some(1), 10)
        .await
        .expect("chunk search");
    assert_eq!(chunk_hits.len(), 1);
    assert_eq!(chunk_hits[0].chunk_id, 1000);

    // Deleting from the exploration space leaves the others untouched.
    explorations.delete(20).unwrap();
    assert!(explorations.is_empty());
    assert_eq!(questions.len(), 1);
}
