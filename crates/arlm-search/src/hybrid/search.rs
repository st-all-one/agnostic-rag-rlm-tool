use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};
use arlm_storage::Storage;

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
    /// - Tier 3 (LlmRerank): Tier 2 + LLM rerank (requires a configured
    ///   `llm_backend` via [`Self::with_llm_backend`] and `storage` to hydrate
    ///   candidate snippets)
    ///
    /// Pass `chunk_ages` (`chunk_id` -> age in hours) to apply decay after fusion.
    /// Pass `None` to skip decay.
    /// Pass `storage` (`Some`) to enable Tier 3 LLM reranking; when `None`, the
    /// fused results are returned as-is even if a backend is configured.
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
        storage: Option<&Storage>,
    ) -> Result<Vec<HybridResult>> {
        let start = Instant::now();
        let k = options.top_k;

        let mut scores: HashMap<i64, f32> = HashMap::with_capacity(k * 3);

        // Tier 0: BM25 always runs
        let tier_start = Instant::now();
        let bm25_results = self.bm25.search(query, buffer_id, k * 3)?;
        for (rank, result) in bm25_results.iter().enumerate() {
            *scores.entry(result.chunk_id).or_insert(0.0) += rrf_score(rank, self.rrf_k);
        }
        tracing::debug!(
            elapsed_ms = tier_start.elapsed().as_millis(),
            results = bm25_results.len(),
            "tier=fts fused"
        );

        // Tier 1+: entity search
        if matches!(
            options.tier,
            SearchTier::Entity | SearchTier::Vector | SearchTier::LlmRerank
        ) {
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
        if matches!(options.tier, SearchTier::Vector | SearchTier::LlmRerank) {
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

        // Dual-layer recall: also search the `summaries` table when storage is
        // available (gaps #1/#2). Summaries are fused into a separate score map
        // keyed by summary id so they never collide with chunk ids.
        let mut summary_scores: HashMap<i64, f32> = HashMap::new();
        if let Some(storage) = storage {
            let tier_start = Instant::now();
            match storage.search_summaries(query, buffer_id, k * 3) {
                Ok(hits) => {
                    for (rank, hit) in hits.iter().enumerate() {
                        *summary_scores.entry(hit.id).or_insert(0.0) += rrf_score(rank, self.rrf_k);
                    }
                    tracing::debug!(
                        elapsed_ms = tier_start.elapsed().as_millis(),
                        results = hits.len(),
                        "tier=summary fused"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "summary search failed, skipping dual-layer");
                }
            }
        }

        let fusion_start = Instant::now();
        let chunk_results: Vec<HybridResult> = scores
            .into_iter()
            .map(|(chunk_id, score)| HybridResult {
                chunk_id,
                score,
                is_summary: false,
            })
            .collect();

        let summary_results: Vec<HybridResult> = summary_scores
            .into_iter()
            .map(|(chunk_id, score)| HybridResult {
                chunk_id,
                score,
                is_summary: true,
            })
            .collect();

        let mut results: Vec<HybridResult> =
            chunk_results.into_iter().chain(summary_results).collect();

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

        // Tier 3: LLM rerank (requires a configured backend + storage)
        if matches!(options.tier, SearchTier::LlmRerank) {
            if let Some(storage) = storage {
                match self.rerank_with_llm(storage, query, results.clone()).await {
                    Ok(reranked) => results = reranked,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "LLM rerank failed, falling back to fused results"
                        );
                    }
                }
            } else {
                tracing::warn!("LLM rerank requires storage access, falling back to fused results");
            }
        }

        tracing::info!(
            tier = %options.tier,
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "hybrid search completed"
        );

        Ok(results)
    }
}
