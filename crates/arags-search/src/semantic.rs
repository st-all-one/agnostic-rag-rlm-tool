use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use arags_storage::VectorStore;
use tracing::info;

use crate::types::SemanticResult;

pub struct SemanticSearch {
    store: Arc<VectorStore>,
}

impl SemanticSearch {
    #[must_use]
    pub fn new(store: Arc<VectorStore>) -> Self {
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

        info!(
            buffer_id,
            results_count = results.len(),
            duration_ms = %start.elapsed().as_millis(),
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

        info!(
            results_count = results.len(),
            duration_ms = %start.elapsed().as_millis(),
            "semantic search_all completed"
        );

        Ok(results)
    }
}
