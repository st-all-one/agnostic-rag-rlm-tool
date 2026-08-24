#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss
)]

use std::sync::Arc;

use arags_search::semantic::SemanticSearch;
use arags_storage::lance::vectors::{VectorEntry, VectorStore};
use tempfile::TempDir;

// Matches the default vector-store dimensionality (all-MiniLM-L6-v2).
const DIMS: usize = 384;

async fn setup_store() -> (SemanticSearch, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(VectorStore::open(tmp.path()).await.unwrap());

    let entries: Vec<VectorEntry> = (0..3)
        .map(|i| VectorEntry {
            chunk_id: i,
            buffer_id: 0,
            vector: vec![i as f32; DIMS],
        })
        .collect();

    store.insert_vectors(&entries).await.unwrap();
    (SemanticSearch::new(store), tmp)
}

#[tokio::test]
async fn test_semantic_search() {
    let (search, _tmp) = setup_store().await;
    let query = vec![0.0_f32; DIMS];
    let results = search.search(&query, 0, 10).await.unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].chunk_id, 0);
}

#[tokio::test]
async fn test_semantic_search_all() {
    let (search, _tmp) = setup_store().await;
    let query = vec![1.0_f32; DIMS];
    let results = search.search_all(&query, 10).await.unwrap();
    assert!(!results.is_empty());
}

#[tokio::test]
async fn test_semantic_search_buffer_filter() {
    let tmp = TempDir::new().unwrap();
    let store = VectorStore::open(tmp.path()).await.unwrap();

    let entries = vec![
        VectorEntry {
            chunk_id: 0,
            buffer_id: 1,
            vector: vec![1.0; DIMS],
        },
        VectorEntry {
            chunk_id: 1,
            buffer_id: 2,
            vector: vec![2.0; DIMS],
        },
    ];
    store.insert_vectors(&entries).await.unwrap();

    let search = SemanticSearch::new(Arc::new(store));
    let query = vec![1.0_f32; DIMS];
    let results = search.search(&query, 1, 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, 0);
}
