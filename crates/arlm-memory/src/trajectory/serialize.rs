//! Task hashing and decomposition-tree flattening helpers.

use sha2::{Digest, Sha256};

use crate::trajectory::DecompositionNode;

/// Compute a deterministic SHA-256 hex hash of a task string.
#[must_use]
pub fn compute_task_hash(task: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(task.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

/// Flatten a decomposition tree into an ordered list of step descriptions.
#[must_use]
pub fn flatten_decomposition(node: &DecompositionNode) -> Vec<String> {
    let mut steps = Vec::new();
    if !node.description.is_empty() {
        steps.push(node.description.clone());
    }
    for child in &node.children {
        steps.extend(flatten_decomposition(child));
    }
    steps
}
