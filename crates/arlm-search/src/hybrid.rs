use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};

use crate::bm25::Bm25Search;
use crate::semantic::SemanticSearch;
use crate::types::{HybridResult, SearchOptions, SearchTier};

pub struct HybridSearch {
    bm25: Bm25Search,
    semantic: Option<SemanticSearch>,
    rrf_k: f32,
}

impl HybridSearch {
    #[must_use]
    pub fn new(bm25: Bm25Search, semantic: Option<SemanticSearch>) -> Self {
        Self {
            bm25,
            semantic,
            rrf_k: 60.0,
        }
    }

    #[must_use]
    pub fn bm25(&self) -> &Bm25Search {
        &self.bm25
    }

    /// Search using BM25 only (tier 0).
    ///
    /// # Errors
    ///
    /// Returns an error if the BM25 query fails.
    pub fn search_fts(
        &self,
        query: &str,
        buffer_id: i64,
        top_k: usize,
    ) -> Result<Vec<HybridResult>> {
        let start = Instant::now();

        let bm25_results = self.bm25.search(query, buffer_id, top_k)?;

        let results: Vec<HybridResult> = bm25_results
            .into_iter()
            .map(|r| HybridResult {
                chunk_id: r.chunk_id,
                #[allow(clippy::cast_possible_truncation)]
                score: r.score as f32,
            })
            .collect();

        tracing::info!(
            tier = "fts",
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "hybrid search completed"
        );

        Ok(results)
    }

    /// Hybrid search with tier support and RRF fusion.
    ///
    /// # Errors
    ///
    /// Returns an error if the BM25 or semantic query fails.
    pub async fn search(
        &self,
        query: &str,
        query_vector: Option<&[f32]>,
        buffer_id: i64,
        options: &SearchOptions,
    ) -> Result<Vec<HybridResult>> {
        let start = Instant::now();
        let k = options.top_k;

        let mut scores: HashMap<i64, f32> = HashMap::new();

        let bm25_results = self.bm25.search(query, buffer_id, k * 3)?;
        for (rank, result) in bm25_results.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let rrf_score = 1.0 / (self.rrf_k + rank as f32 + 1.0);
            *scores
                .entry(result.chunk_id)
                .or_insert(0.0) += rrf_score;
        }

        if matches!(options.tier, SearchTier::Vector | SearchTier::Entity) {
            if let Some(sem) = &self.semantic {
                if let Some(vec) = query_vector {
                    match sem.search(vec, buffer_id, k * 3).await {
                        Ok(sem_results) => {
                            for (rank, result) in sem_results.iter().enumerate() {
                                #[allow(clippy::cast_precision_loss)]
                                let rrf_score = 1.0 / (self.rrf_k + rank as f32 + 1.0);
                                let cid = i64::try_from(result.chunk_id)
                                    .context("chunk_id overflow")?;
                                *scores.entry(cid).or_insert(0.0) += rrf_score;
                            }
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

        tracing::info!(
            tier = %options.tier,
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "hybrid search completed"
        );

        Ok(results)
    }

    /// Reciprocal Rank Fusion of multiple result lists.
    #[must_use]
    pub fn rrf_fuse(results_list: &[Vec<HybridResult>], top_k: usize, k: f32) -> Vec<HybridResult> {
        let mut scores: HashMap<i64, f32> = HashMap::new();

        for results in results_list {
            for (rank, result) in results.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let rrf_score = 1.0 / (k + rank as f32 + 1.0);
                *scores
                    .entry(result.chunk_id)
                    .or_insert(0.0) += rrf_score;
            }
        }

        let mut fused: Vec<HybridResult> = scores
            .into_iter()
            .map(|(chunk_id, score)| HybridResult { chunk_id, score })
            .collect();

        fused.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        fused.truncate(top_k);
        fused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_fuse_single_list() {
        let results = vec![
            HybridResult { chunk_id: 1, score: 0.9 },
            HybridResult { chunk_id: 2, score: 0.8 },
        ];

        let fused = HybridSearch::rrf_fuse(&[results], 10, 60.0);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].chunk_id, 1);
    }

    #[test]
    fn test_rrf_fuse_multiple_lists() {
        let list1 = vec![
            HybridResult { chunk_id: 1, score: 0.9 },
            HybridResult { chunk_id: 2, score: 0.8 },
            HybridResult { chunk_id: 3, score: 0.7 },
        ];

        let list2 = vec![
            HybridResult { chunk_id: 2, score: 0.95 },
            HybridResult { chunk_id: 1, score: 0.85 },
            HybridResult { chunk_id: 4, score: 0.75 },
        ];

        let fused = HybridSearch::rrf_fuse(&[list1, list2], 10, 60.0);

        assert_eq!(fused.len(), 4);

        let chunk_ids: Vec<i64> = fused.iter().map(|r| r.chunk_id).collect();
        assert!(chunk_ids.contains(&1));
        assert!(chunk_ids.contains(&2));
        assert!(chunk_ids.contains(&3));
        assert!(chunk_ids.contains(&4));

        let scores: Vec<f32> = fused.iter().map(|r| r.score).collect();
        assert!(scores[0] >= scores[1]);
        assert!(scores[1] >= scores[2]);
    }

    #[test]
    fn test_rrf_fuse_top_k_limit() {
        let results: Vec<HybridResult> = (0..20)
            .map(|i| HybridResult {
                chunk_id: i,
                score: 1.0 - i as f32 * 0.01,
            })
            .collect();

        let fused = HybridSearch::rrf_fuse(&[results], 5, 60.0);
        assert_eq!(fused.len(), 5);
    }

    #[test]
    fn test_rrf_fuse_empty() {
        let fused = HybridSearch::rrf_fuse(&[], 10, 60.0);
        assert!(fused.is_empty());
    }

    #[test]
    fn test_rrf_fuse_disjoint_results() {
        let list1 = vec![HybridResult { chunk_id: 1, score: 0.9 }];
        let list2 = vec![HybridResult { chunk_id: 2, score: 0.9 }];

        let fused = HybridSearch::rrf_fuse(&[list1, list2], 10, 60.0);
        assert_eq!(fused.len(), 2);

        let s1 = fused.iter().find(|r| r.chunk_id == 1).unwrap().score;
        let s2 = fused.iter().find(|r| r.chunk_id == 2).unwrap().score;
        assert!((s1 - s2).abs() < f32::EPSILON);
    }

    #[test]
    fn test_rrf_fuse_overlapping_high_rank_wins() {
        let list1 = vec![
            HybridResult { chunk_id: 1, score: 0.9 },
            HybridResult { chunk_id: 2, score: 0.8 },
        ];

        let list2 = vec![
            HybridResult { chunk_id: 1, score: 0.95 },
            HybridResult { chunk_id: 3, score: 0.7 },
        ];

        let fused = HybridSearch::rrf_fuse(&[list1, list2], 10, 60.0);

        assert_eq!(fused[0].chunk_id, 1);
    }
}
