//! Quorum / security configuration (Cluster B, issue `agnostic-rag-rlm-tool-a5d7`).

use serde::{Deserialize, Serialize};

/// How to fuse agreeing candidate submissions into the accepted answer
/// (issue `agnostic-rag-rlm-tool-a5d7`). The selection of *which* candidates agree is
/// decided by the quorum cosine-similarity threshold (`quorum_sim_threshold`,
/// implemented in later issues `6d97`/`64af`); this enum only picks the merge
/// strategy once a quorum exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FusionStrategy {
    /// Take the single candidate closest to the consensus embedding.
    #[default]
    Consensus,
    /// Embedding average of the agreeing candidates (then nearest text).
    Average,
    /// The longest candidate text among the agreeing set.
    Longest,
}

/// Quorum / security configuration (Cluster B, issue `agnostic-rag-rlm-tool-a5d7`).
///
/// Drives the volunteer fan-out and candidate-submission decision pipeline.
/// The decision algorithm itself lives in later issues (`6d97`/`64af`); this
/// struct only carries the tunables.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QuorumConfig {
    /// Number of volunteers a job is fanned out to.
    pub n: usize,
    /// Cosine similarity threshold above which two candidates are "in
    /// agreement".
    pub quorum_sim_threshold: f64,
    /// How to fuse agreeing candidates into the accepted answer.
    pub fusion_strategy: FusionStrategy,
    /// Strikes a volunteer accumulates before being deprioritized/banned.
    pub strikes_limit: u32,
}

fn default_quorum_n() -> usize {
    3
}
fn default_quorum_sim_threshold() -> f64 {
    0.85
}
fn default_quorum_strikes_limit() -> u32 {
    3
}

impl Default for QuorumConfig {
    fn default() -> Self {
        Self {
            n: default_quorum_n(),
            quorum_sim_threshold: default_quorum_sim_threshold(),
            fusion_strategy: FusionStrategy::default(),
            strikes_limit: default_quorum_strikes_limit(),
        }
    }
}
