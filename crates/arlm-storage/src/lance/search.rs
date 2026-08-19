use anyhow::Result;

use super::vectors::VectorStore;

/// Search result with distance.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk_id: u64,
    pub distance: f32,
}

impl VectorStore {
    /// Search for similar vectors and return structured results.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying vector search fails.
    pub async fn search_similar(
        &self,
        query_vector: &[f32],
        buffer_id: Option<u64>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let raw_results = self.search(query_vector, buffer_id, limit).await?;

        Ok(raw_results
            .into_iter()
            .map(|(chunk_id, distance)| SearchResult { chunk_id, distance })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::lance::vectors::{VectorEntry, VectorStore};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_search_similar() {
        let tmp = TempDir::new().unwrap();
        let store = VectorStore::open(tmp.path()).await.unwrap();

        let entries: Vec<VectorEntry> = (0..5)
            .map(|i| VectorEntry {
                chunk_id: i,
                buffer_id: 0,
                vector: vec![i as f32; 1024],
            })
            .collect();

        store.insert_vectors(&entries).await.unwrap();

        let query = vec![0.0_f32; 1024];
        let results = store.search_similar(&query, None, 3).await.unwrap();

        assert!(!results.is_empty());
        assert!(results.len() <= 3);
        assert_eq!(results[0].chunk_id, 0);
    }
}
