use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};
use arags_storage::Storage;

use super::HybridSearch;

use crate::types::HybridResult;

impl HybridSearch {
    /// Apply salience decay to results using chunk ages (in hours).
    ///
    /// Mutates scores in-place: `score *= exp(-lambda * age_hours)`.
    /// Re-sorts by decayed score and returns the results.
    #[must_use]
    pub fn apply_decay(
        &self,
        mut results: Vec<HybridResult>,
        chunk_ages: &HashMap<i64, f32>,
    ) -> Vec<HybridResult> {
        if !self.decay.enabled || chunk_ages.is_empty() {
            return results;
        }

        for r in &mut results {
            if let Some(&age) = chunk_ages.get(&r.chunk_id) {
                r.score = self.decay.score(r.score, age);
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results
    }

    /// Search using BM25 only (tier 0), with optional salience decay.
    ///
    /// # Errors
    ///
    /// Returns an error if the BM25 query fails.
    pub fn search_fts(
        &self,
        query: &str,
        buffer_id: i64,
        top_k: usize,
        chunk_ages: Option<&HashMap<i64, f32>>,
    ) -> Result<Vec<HybridResult>> {
        let start = Instant::now();

        let bm25_results = self.bm25.search(query, buffer_id, top_k)?;

        let mut results: Vec<HybridResult> = bm25_results
            .into_iter()
            .map(|r| HybridResult {
                chunk_id: r.chunk_id,
                #[allow(clippy::cast_possible_truncation)]
                score: r.score as f32,
            })
            .collect();

        if let Some(ages) = chunk_ages {
            results = self.apply_decay(results, ages);
        }

        tracing::info!(
            tier = "fts",
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "hybrid search completed"
        );

        Ok(results)
    }

    /// Search across all buffers (projects) with RRF fusion.
    ///
    /// Iterates all buffers in the database, runs BM25 search on each,
    /// and fuses results using Reciprocal Rank Fusion.
    ///
    /// # Errors
    ///
    /// Returns an error if listing buffers fails.
    pub fn search_all(
        &self,
        query: &str,
        top_k: usize,
        storage: &Storage,
    ) -> Result<Vec<HybridResult>> {
        let buffers = storage.list_buffers().context("failed to list buffers")?;

        if buffers.is_empty() {
            return Ok(Vec::new());
        }

        let mut chunk_lists: Vec<Vec<HybridResult>> = Vec::with_capacity(buffers.len());

        for buffer in &buffers {
            match self.bm25.search(query, buffer.id, top_k * 2) {
                Ok(bm25_results) => {
                    let results: Vec<HybridResult> = bm25_results
                        .into_iter()
                        .map(|r| HybridResult {
                            chunk_id: r.chunk_id,
                            #[allow(clippy::cast_possible_truncation)]
                            score: r.score as f32,
                        })
                        .collect();
                    chunk_lists.push(results);
                }
                Err(e) => {
                    tracing::warn!(
                        buffer = %buffer.name,
                        error = %e,
                        "BM25 search failed on buffer, skipping"
                    );
                }
            }
        }

        let mut fused: Vec<HybridResult> = Self::rrf_fuse(&chunk_lists, top_k, self.rrf_k);
        fused.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        fused.truncate(top_k);

        tracing::info!(
            buffers = buffers.len(),
            results_count = fused.len(),
            "cross-project search completed"
        );

        Ok(fused)
    }
}
