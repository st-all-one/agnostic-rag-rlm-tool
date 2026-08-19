use std::time::Instant;

use anyhow::{Context, Result};
use arlm_storage::VectorStore;

use crate::types::SemanticResult;

pub struct SemanticSearch {
    store: VectorStore,
}

impl SemanticSearch {
    #[must_use]
    pub fn new(store: VectorStore) -> Self {
        Self { store }
    }

    /// Search for similar vectors with `buffer_id` filter.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector search query fails.
    pub async fn search(
        &self,
        query_vector: &[f32],
        buffer_id: i64,
        top_k: usize,
    ) -> Result<Vec<SemanticResult>> {
        let start = Instant::now();

        let bid = u64::try_from(buffer_id).context("buffer_id overflow")?;
        let raw = self
            .store
            .search_similar(query_vector, Some(bid), top_k * 2)
            .await
            .context("vector search failed")?;

        let results: Vec<SemanticResult> = raw
            .into_iter()
            .map(|r| SemanticResult {
                chunk_id: r.chunk_id,
                score: 1.0 / (1.0 + r.distance),
            })
            .collect();

        tracing::info!(
            buffer_id,
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "semantic search completed"
        );

        Ok(results)
    }

    /// Search for similar vectors across all buffers.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector search query fails.
    pub async fn search_all(
        &self,
        query_vector: &[f32],
        top_k: usize,
    ) -> Result<Vec<SemanticResult>> {
        let start = Instant::now();

        let raw = self
            .store
            .search_similar(query_vector, None, top_k * 2)
            .await
            .context("vector search failed")?;

        let results: Vec<SemanticResult> = raw
            .into_iter()
            .map(|r| SemanticResult {
                chunk_id: r.chunk_id,
                score: 1.0 / (1.0 + r.distance),
            })
            .collect();

        tracing::info!(
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "semantic search_all completed"
        );

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlm_storage::lance::vectors::{VectorEntry, VectorStore};
    use tempfile::TempDir;

    const DIMS: usize = 1024;

    async fn setup_store() -> (SemanticSearch, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = VectorStore::open(tmp.path()).await.unwrap();

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

        let search = SemanticSearch::new(store);
        let query = vec![1.0_f32; DIMS];
        let results = search.search(&query, 1, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, 0);
    }
}
