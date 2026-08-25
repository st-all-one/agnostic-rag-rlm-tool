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

/// Deterministic content hash for a chunk's text (SHA-256, hex-encoded).
///
/// Shared by the client (digest-once `StoreAnswer.source_hashes`) and the
/// server (staleness invalidation when indexed chunks change), so both sides
/// compute identical hashes without a storage dependency (plan 020: the CLI
/// is a pure gRPC client and never opens local storage).
#[must_use]
pub fn chunk_content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests;
