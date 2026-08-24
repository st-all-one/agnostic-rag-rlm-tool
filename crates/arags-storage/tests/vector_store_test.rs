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

use arags_storage::VectorStore;
use arags_storage::lance::vectors::VectorEntry;
use tempfile::TempDir;

// Matches the default vector-store dimensionality (all-MiniLM-L6-v2).
const DIMS: usize = 384;

fn vec_of(value: f32) -> Vec<f32> {
    vec![value; DIMS]
}

#[tokio::test]
async fn test_open_and_count_empty() {
    let tmp = TempDir::new().unwrap();
    let store = VectorStore::open(tmp.path()).await.unwrap();
    assert_eq!(store.count().await, 0);
}

#[tokio::test]
async fn test_insert_and_count() {
    let tmp = TempDir::new().unwrap();
    let store = VectorStore::open(tmp.path()).await.unwrap();

    let entries: Vec<VectorEntry> = (0..5)
        .map(|i| VectorEntry {
            chunk_id: i,
            buffer_id: 0,
            vector: vec_of(i as f32),
        })
        .collect();

    store.insert_vectors(&entries).await.unwrap();
    assert_eq!(store.count().await, 5);
}

#[tokio::test]
async fn test_insert_empty_is_noop() {
    let tmp = TempDir::new().unwrap();
    let store = VectorStore::open(tmp.path()).await.unwrap();
    store.insert_vectors(&[]).await.unwrap();
    assert_eq!(store.count().await, 0);
}

#[tokio::test]
async fn test_search_returns_nearest() {
    let tmp = TempDir::new().unwrap();
    let store = VectorStore::open(tmp.path()).await.unwrap();

    let entries: Vec<VectorEntry> = (0..3)
        .map(|i| VectorEntry {
            chunk_id: i,
            buffer_id: 0,
            vector: vec_of(i as f32),
        })
        .collect();
    store.insert_vectors(&entries).await.unwrap();

    let results = store.search_similar(&vec_of(0.0), None, 10).await.unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].chunk_id, 0);
}

#[tokio::test]
async fn test_search_with_buffer_filter() {
    let tmp = TempDir::new().unwrap();
    let store = VectorStore::open(tmp.path()).await.unwrap();

    let entries = vec![
        VectorEntry {
            chunk_id: 0,
            buffer_id: 1,
            vector: vec_of(1.0),
        },
        VectorEntry {
            chunk_id: 1,
            buffer_id: 2,
            vector: vec_of(2.0),
        },
    ];
    store.insert_vectors(&entries).await.unwrap();

    let results = store
        .search_similar(&vec_of(1.0), Some(1), 10)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, 0);
}

#[tokio::test]
async fn test_persistence_across_reopen() {
    let tmp = TempDir::new().unwrap();
    {
        let store = VectorStore::open(tmp.path()).await.unwrap();
        let entries: Vec<VectorEntry> = (0..4)
            .map(|i| VectorEntry {
                chunk_id: i,
                buffer_id: 0,
                vector: vec_of(i as f32),
            })
            .collect();
        store.insert_vectors(&entries).await.unwrap();
    }
    let store = VectorStore::open(tmp.path()).await.unwrap();
    assert_eq!(store.count().await, 4);
    let results = store.search_similar(&vec_of(0.0), None, 3).await.unwrap();
    assert_eq!(results[0].chunk_id, 0);
}
