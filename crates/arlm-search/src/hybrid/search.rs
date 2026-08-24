use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};

use super::HybridSearch;
use super::rrf::rrf_score;
use crate::entity::EntitySearch;
use crate::types::{HybridResult, SearchOptions, SearchTier};

impl HybridSearch {
    /// Hybrid search with tier support, RRF fusion, and optional salience decay.
    ///
    /// - Tier 0 (Fts): BM25 only
    /// - Tier 1 (Entity, default): BM25 + entity RRF
    /// - Tier 2 (Vector): BM25 + entity + vector RRF
    ///
    /// Pass `chunk_ages` (`chunk_id` -> age in hours) to apply decay after fusion.
    /// Pass `None` to skip decay.
    ///
    /// # Errors
    ///
    /// Returns an error if the BM25, entity, or semantic query fails.
    #[allow(clippy::too_many_lines)]
    pub async fn search(
        &self,
        query: &str,
        query_vector: Option<&[f32]>,
        buffer_id: i64,
        options: &SearchOptions,
        chunk_ages: Option<&HashMap<i64, f32>>,
    ) -> Result<Vec<HybridResult>> {
        let start = Instant::now();
        let k = options.top_k;

        let mut scores: HashMap<i64, f32> = HashMap::with_capacity(k * 3);

        // Tier 0: BM25 always runs (but a parse error must not abort the
        // whole search — degrade to the other tiers like entity/semantic do).
        let tier_start = Instant::now();
        let bm25_results = match self.bm25.search(query, buffer_id, k * 3) {
            Ok(results) if !results.is_empty() => results,
            // A natural-language question rarely matches every token (implicit
            // AND), so retry once with an OR of the sanitized tokens to recover
            // lexical recall instead of leaving the lexical tier dead.
            Ok(_) => {
                let or_query = arlm_storage::fts::sanitize_query(query)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" OR ");
                if or_query.split_whitespace().count() > 1 {
                    match self.bm25.search(&or_query, buffer_id, k * 3) {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(error = %e, "bm25 OR-fallback failed, continuing without lexical tier");
                            Vec::new()
                        }
                    }
                } else {
                    Vec::new()
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "bm25 search failed, continuing without lexical tier");
                Vec::new()
            }
        };
        for (rank, result) in bm25_results.iter().enumerate() {
            *scores.entry(result.chunk_id).or_insert(0.0) += rrf_score(rank, self.rrf_k);
        }
        tracing::debug!(
            elapsed_ms = tier_start.elapsed().as_millis(),
            results = bm25_results.len(),
            "tier=fts fused"
        );

        // Tier 1+: entity search
        if matches!(options.tier, SearchTier::Entity | SearchTier::Vector) {
            if let Some(entity_search) = &self.entity {
                let entities = EntitySearch::extract_query_entities(query);
                if !entities.is_empty() {
                    let tier_start = Instant::now();
                    match entity_search.search(&entities, buffer_id, k * 3) {
                        Ok(entity_results) => {
                            for (rank, result) in entity_results.iter().enumerate() {
                                *scores.entry(result.chunk_id).or_insert(0.0) +=
                                    rrf_score(rank, self.rrf_k);
                            }
                            tracing::debug!(
                                elapsed_ms = tier_start.elapsed().as_millis(),
                                results = entity_results.len(),
                                "tier=entity fused"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "entity search failed, falling back to BM25 only"
                            );
                        }
                    }
                }
            }
        }

        // Tier 2+: vector search
        if options.tier == SearchTier::Vector {
            if let Some(sem) = &self.semantic {
                if let Some(vec) = query_vector {
                    let tier_start = Instant::now();
                    match sem.search(vec, buffer_id, k * 3).await {
                        Ok(sem_results) => {
                            for (rank, result) in sem_results.iter().enumerate() {
                                let cid =
                                    i64::try_from(result.chunk_id).context("chunk_id overflow")?;
                                *scores.entry(cid).or_insert(0.0) += rrf_score(rank, self.rrf_k);
                            }
                            tracing::debug!(
                                elapsed_ms = tier_start.elapsed().as_millis(),
                                results = sem_results.len(),
                                "tier=vector fused"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "semantic search failed, falling back to BM25 only"
                            );
                        }
                    }
                }
            }
        }

        let fusion_start = Instant::now();
        let mut results: Vec<HybridResult> = scores
            .into_iter()
            .map(|(chunk_id, score)| HybridResult { chunk_id, score })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(k);

        if let Some(ages) = chunk_ages {
            results = self.apply_decay(results, ages);
            results.truncate(k);
        }
        tracing::debug!(
            elapsed_ms = fusion_start.elapsed().as_millis(),
            "fusion + decay applied"
        );

        tracing::info!(
            tier = %options.tier,
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "hybrid search completed"
        );

        Ok(results)
    }
}
