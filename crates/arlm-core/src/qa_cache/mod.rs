//! Adaptive widening engine for the semantic query-answer cache (plan 017).
//!
//! Maps a query's resolved similarity (question cosine) **and** secondary
//! check (provenance Jaccard) onto a digest plan: how many chunks to re-digest
//! on the client and how many provenance chunks to return with the cached
//! answer. Lower similarity → wider context (more chunks), never exceeding
//! `novel_k`. The invariant `provenance_k ≤ digest_k ≤ novel_k` always holds.
//!
//! This module is pure (no storage, no embedder) so it can be unit-tested and
//! reused by both the server (lookup) and the client (digest-once).

use serde::{Deserialize, Serialize};

/// Configurable thresholds for cache lookup and widening.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaThresholds {
    /// Chunks digested on a brand-new question (client). Upper bound.
    pub novel_k: usize,
    /// Provenance chunks returned alongside a cached answer.
    pub provenance_k: usize,
    /// At/above this similarity a hit is a high-confidence near-exact match.
    pub sim_high: f32,
    /// Below this similarity the query is treated as brand new (full digest).
    pub sim_floor: f32,
    /// Descending similarity boundaries for widening tiers.
    pub tier_steps: Vec<f32>,
    /// Minimum provenance Jaccard for a hit to pass the secondary check.
    pub jaccard_min: f32,
}

impl Default for QaThresholds {
    fn default() -> Self {
        Self {
            novel_k: 20,
            provenance_k: 5,
            sim_high: 0.90,
            sim_floor: 0.40,
            tier_steps: vec![0.90, 0.80, 0.70, 0.60, 0.50],
            jaccard_min: 0.5,
        }
    }
}

/// A digest plan produced by [`resolve_plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaPlan {
    /// Chunks the client should digest (`≤ novel_k`).
    pub digest_k: usize,
    /// Provenance chunks to return with the answer (`≤ digest_k`).
    pub provenance_k: usize,
    /// Whether this is a MISS (full fresh digest + new cache entry).
    pub is_miss: bool,
    /// Tier index: `-1` for miss, `0..=tier_steps.len()-1` for hits.
    pub tier: i32,
}

impl QaPlan {
    /// Whether this is a high-confidence near-exact hit (top tier).
    #[must_use]
    pub fn is_top_tier(&self) -> bool {
        self.tier == 0
    }
}

// Per-tier digest/provenance schedule (index aligns with `tier_steps`).
const DIGEST_SCHEDULE: [usize; 5] = [10, 12, 13, 15, 18];
const PROV_SCHEDULE: [usize; 5] = [5, 6, 7, 8, 10];

/// Resolve a similarity + secondary-check Jaccard into a digest plan.
///
/// # Panics
///
/// Never panics; clamps gracefully when `tier_steps` is shorter than the
/// schedule arrays.
#[must_use]
pub fn resolve_plan(similarity: f32, jaccard: f32, t: &QaThresholds) -> QaPlan {
    // Below the floor → brand new question.
    if similarity < t.sim_floor {
        return QaPlan {
            digest_k: t.novel_k,
            provenance_k: t.provenance_k,
            is_miss: true,
            tier: -1,
        };
    }

    // Find the highest descending tier step the similarity still meets.
    let tier_idx = t.tier_steps.iter().position(|&step| similarity >= step);

    // Similarity is above the floor but below the lowest tier step:
    // treat as a near-miss (full fresh digest).
    let Some(tier_idx) = tier_idx else {
        return QaPlan {
            digest_k: t.novel_k,
            provenance_k: t.provenance_k,
            is_miss: true,
            tier: -1,
        };
    };

    // Secondary check defeats false positives (e.g. "login" vs "logout").
    if jaccard < t.jaccard_min {
        return QaPlan {
            digest_k: t.novel_k,
            provenance_k: t.provenance_k,
            is_miss: true,
            tier: -1,
        };
    }

    let digest_k = DIGEST_SCHEDULE
        .get(tier_idx)
        .copied()
        .unwrap_or(t.novel_k)
        .min(t.novel_k);
    let provenance_k = PROV_SCHEDULE
        .get(tier_idx)
        .copied()
        .unwrap_or(t.provenance_k)
        .min(digest_k)
        .max(1);

    QaPlan {
        digest_k,
        provenance_k,
        is_miss: false,
        tier: i32::try_from(tier_idx).unwrap_or(-1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miss_below_floor() {
        let t = QaThresholds::default();
        let p = resolve_plan(0.2, 0.0, &t);
        assert!(p.is_miss);
        assert_eq!(p.digest_k, t.novel_k);
    }

    #[test]
    fn false_positive_blocked_by_jaccard() {
        let t = QaThresholds::default();
        // "login" vs "logout" can be cos-similar but disjoint provenance.
        let p = resolve_plan(0.92, 0.1, &t);
        assert!(p.is_miss);
    }

    #[test]
    fn top_tier_hit() {
        let t = QaThresholds::default();
        let p = resolve_plan(0.95, 0.8, &t);
        assert!(!p.is_miss);
        assert!(p.is_top_tier());
        assert_eq!(p.digest_k, 10);
        assert_eq!(p.provenance_k, 5);
        assert!(p.provenance_k <= p.digest_k);
        assert!(p.digest_k <= t.novel_k);
    }

    #[test]
    fn widening_lower_tier() {
        let t = QaThresholds::default();
        let p = resolve_plan(0.65, 0.6, &t);
        assert!(!p.is_miss);
        assert!(p.digest_k >= 10);
        assert!(p.provenance_k <= p.digest_k);
    }

    #[test]
    fn invariant_holds_at_every_tier() {
        let t = QaThresholds::default();
        for s in [0.50, 0.55, 0.62, 0.71, 0.83, 0.91, 0.99] {
            let p = resolve_plan(s, 0.9, &t);
            if !p.is_miss {
                assert!(p.provenance_k <= p.digest_k);
                assert!(p.digest_k <= t.novel_k);
            }
        }
    }
}
