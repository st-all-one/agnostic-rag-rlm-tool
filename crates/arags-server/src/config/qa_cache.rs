//! Semantic query-answer cache (plan 017) tunables.

use serde::{Deserialize, Serialize};

/// Semantic query-answer cache (plan 017) tunables. All thresholds and
/// dimensionalities are configurable without a rebuild.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaCacheConfig {
    /// Chunks digested on a brand-new question (client). Upper bound.
    #[serde(default = "default_novel_k")]
    pub novel_k: usize,

    /// Provenance chunks returned alongside a cached answer.
    #[serde(default = "default_provenance_k")]
    pub provenance_k: usize,

    /// At/above this similarity a hit is a high-confidence near-exact match.
    #[serde(default = "default_sim_high")]
    pub sim_high: f32,

    /// Below this similarity the query is treated as brand new (full digest).
    #[serde(default = "default_sim_floor")]
    pub sim_floor: f32,

    /// Descending similarity boundaries for widening tiers.
    #[serde(default = "default_tier_steps")]
    pub tier_steps: Vec<f32>,

    /// Minimum provenance Jaccard for a hit to pass the secondary check.
    #[serde(default = "default_jaccard_min")]
    pub jaccard_min: f32,

    /// Dimensionality of the question embedding space.
    #[serde(default = "default_question_dims")]
    pub question_vector_dims: usize,

    /// Max cached entries kept per project before weighted-LRU eviction.
    #[serde(default = "default_max_entries")]
    pub max_entries_per_project: usize,

    /// Age half-life (ms) for weighted-LRU eviction scoring.
    #[serde(default = "default_eviction_lambda_ms")]
    pub eviction_lambda_ms: i64,

    /// Background eviction interval (ms). 0 disables the worker.
    #[serde(default = "default_eviction_interval_ms")]
    pub eviction_interval_ms: u64,
}

impl Default for QaCacheConfig {
    fn default() -> Self {
        Self {
            novel_k: default_novel_k(),
            provenance_k: default_provenance_k(),
            sim_high: default_sim_high(),
            sim_floor: default_sim_floor(),
            tier_steps: default_tier_steps(),
            jaccard_min: default_jaccard_min(),
            question_vector_dims: default_question_dims(),
            max_entries_per_project: default_max_entries(),
            eviction_lambda_ms: default_eviction_lambda_ms(),
            eviction_interval_ms: default_eviction_interval_ms(),
        }
    }
}

fn default_novel_k() -> usize {
    20
}
fn default_provenance_k() -> usize {
    5
}
fn default_sim_high() -> f32 {
    0.90
}
fn default_sim_floor() -> f32 {
    0.40
}
fn default_tier_steps() -> Vec<f32> {
    vec![0.90, 0.80, 0.70, 0.60, 0.50]
}
fn default_jaccard_min() -> f32 {
    0.5
}
fn default_question_dims() -> usize {
    arags_embedding::embedder::minilm::HIDDEN_SIZE
}
fn default_max_entries() -> usize {
    1_000
}
fn default_eviction_lambda_ms() -> i64 {
    7 * 24 * 60 * 60 * 1_000
}
fn default_eviction_interval_ms() -> u64 {
    60_000
}
