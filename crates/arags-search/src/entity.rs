use std::time::Instant;

use anyhow::{Context, Result};
use arags_storage::Storage;
use tracing::info;

use crate::types::EntityResult;

/// Deterministic entity search using regex extraction + FTS5.
///
/// Entities are extracted from query text using regex patterns (function names,
/// imports, identifiers) and matched against pre-indexed chunk entities via
/// `SQLite` FTS5. This tier requires no embeddings or LLM.
pub struct EntitySearch {
    storage: Storage,
}

impl EntitySearch {
    /// Create a new entity search instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the entities FTS5 table cannot be created.
    pub fn new(storage: Storage) -> Result<Self> {
        storage.ensure_entities_fts()?;
        Ok(Self { storage })
    }

    /// Extract entities from a query string using deterministic regex rules.
    #[must_use]
    pub fn extract_query_entities(query: &str) -> Vec<String> {
        Storage::extract_entities(query, "")
    }

    /// Search for chunks matching the extracted entities.
    ///
    /// # Errors
    ///
    /// Returns an error if the entity search query fails.
    pub fn search(
        &self,
        query_entities: &[String],
        buffer_id: i64,
        top_k: usize,
    ) -> Result<Vec<EntityResult>> {
        let start = Instant::now();

        let hits = self
            .storage
            .search_entities(query_entities, buffer_id, top_k)
            .context("entity search failed")?;

        let results: Vec<EntityResult> = hits
            .into_iter()
            .map(|h| EntityResult {
                chunk_id: h.chunk_id,
                #[allow(clippy::cast_possible_truncation)]
                score: h.score as f32,
            })
            .collect();

        info!(
            buffer_id,
            query_entities = ?query_entities,
            results_count = results.len(),
            duration_ms = %start.elapsed().as_millis(),
            "entity search completed"
        );

        Ok(results)
    }

    /// Search entities across all buffers.
    ///
    /// # Errors
    ///
    /// Returns an error if the entity search query fails.
    pub fn search_all(&self, query_entities: &[String], top_k: usize) -> Result<Vec<EntityResult>> {
        let start = Instant::now();

        let hits = self
            .storage
            .search_entities_all(query_entities, top_k)
            .context("entity search_all failed")?;

        let results: Vec<EntityResult> = hits
            .into_iter()
            .map(|h| EntityResult {
                chunk_id: h.chunk_id,
                #[allow(clippy::cast_possible_truncation)]
                score: h.score as f32,
            })
            .collect();

        info!(
            query_entities = ?query_entities,
            results_count = results.len(),
            duration_ms = %start.elapsed().as_millis(),
            "entity search_all completed"
        );

        Ok(results)
    }
}
