use anyhow::{Context, Result};
use lancedb::index::{Index, vector::IvfHnswFlatIndexBuilder};
use tracing::info;

use super::vectors::VectorStore;

impl VectorStore {
    /// Create HNSW index for vector search.
    ///
    /// Uses IVF-HNSW-Flat (unquantized) with parameters from the plan:
    /// - m = 16 (connections per node)
    /// - `ef_construction` = 200 (build-time search width)
    ///
    /// # Errors
    ///
    /// Returns an error if the `LanceDB` index creation fails.
    pub async fn create_index(&self) -> Result<()> {
        info!("creating HNSW index on vector column");

        let builder = IvfHnswFlatIndexBuilder::default()
            .num_edges(16)
            .ef_construction(200);

        self.table
            .create_index(&["vector"], Index::IvfHnswFlat(builder))
            .execute()
            .await
            .context("failed to create HNSW index")?;

        info!("HNSW index created successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::lance::vectors::{VectorEntry, VectorStore};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_create_index() {
        let tmp = TempDir::new().unwrap();
        let store = VectorStore::open(tmp.path()).await.unwrap();

        let entries: Vec<VectorEntry> = (0..100)
            .map(|i| VectorEntry {
                chunk_id: i,
                buffer_id: 0,
                vector: vec![i as f32; 1024],
            })
            .collect();

        store.insert_vectors(&entries).await.unwrap();
        store.create_index().await.unwrap();
    }
}
