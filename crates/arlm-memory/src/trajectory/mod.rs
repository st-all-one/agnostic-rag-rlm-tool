//! Trajectory storage, retrieval, and replay for run strategies.

pub mod serialize;
pub mod store;

pub use serialize::{compute_task_hash, flatten_decomposition};

use serde::{Deserialize, Serialize};

use arlm_storage::Storage;

/// A complete run trajectory capturing the decomposition and results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTrajectory {
    /// Row id.
    pub id: i64,
    /// Owning project name.
    pub project_name: String,
    /// Original task string.
    pub task: String,
    /// SHA-256 hash of the task.
    pub task_hash: String,
    /// Decomposition tree root.
    pub root: DecompositionNode,
    /// Total cost of the run, if tracked.
    pub total_cost: Option<f64>,
    /// Unix timestamp of creation.
    pub created_at: i64,
}

/// A node in the decomposition tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompositionNode {
    /// Step description.
    pub description: String,
    /// Status string (e.g. "completed").
    pub status: String,
    /// Child nodes.
    pub children: Vec<DecompositionNode>,
}

/// Options for finding similar runs.
#[derive(Debug, Clone)]
pub struct FindSimilarOptions {
    /// Similarity score threshold (0.0 - 1.0).
    pub min_score: f32,
    /// Maximum results to return.
    pub top_k: usize,
}

impl Default for FindSimilarOptions {
    fn default() -> Self {
        Self {
            min_score: 0.7,
            top_k: 5,
        }
    }
}

/// The trajectory engine stores and retrieves run strategies.
pub struct TrajectoryEngine {
    storage: Storage,
}
