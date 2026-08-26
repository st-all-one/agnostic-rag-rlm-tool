use std::collections::HashMap;
use std::time::Instant;

use super::HybridSearch;

use crate::types::HybridResult;

/// Reciprocal Rank Fusion score for a result at `rank` (0-based) with constant `k`.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn rrf_score(rank: usize, k: f32) -> f32 {
    1.0 / (k + rank as f32 + 1.0)
}

impl HybridSearch {
    /// Reciprocal Rank Fusion of multiple result lists.
    ///
    /// Each list is treated as an ordered ranking; every item receives
    /// `1 / (k + rank + 1)` added to its aggregated score, and the merged
    /// results are sorted by descending score and truncated to `top_k`.
    #[must_use]
    pub fn rrf_fuse(results_list: &[Vec<HybridResult>], top_k: usize, k: f32) -> Vec<HybridResult> {
        let start = Instant::now();

        let mut scores: HashMap<i64, f32> = HashMap::with_capacity(results_list.len() * top_k);
        for results in results_list {
            for (rank, result) in results.iter().enumerate() {
                *scores.entry(result.chunk_id).or_insert(0.0) += rrf_score(rank, k);
            }
        }

        let mut fused: Vec<HybridResult> = scores
            .into_iter()
            .map(|(chunk_id, score)| HybridResult { chunk_id, score })
            .collect();

        // Deterministic order: score desc, then chunk_id asc so equal-score
        // items do not shuffle between identical queries (HashMap iteration
        // order is randomized).
        fused.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.chunk_id.cmp(&b.chunk_id))
        });
        fused.truncate(top_k);

        tracing::debug!(
            lists = results_list.len(),
            fused = fused.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "rrf_fuse completed"
        );

        fused
    }
}
