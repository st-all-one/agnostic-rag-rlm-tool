//! Flat node representation for persisting run trajectories.
//!
//! Defined here (instead of in `runs.rs`) to keep that module focused on the
//! SQL persistence layer. Avoids a dependency on `arlm-core`.

/// Flat representation of a node for persistence (avoids `arlm-core` dependency).
#[derive(Debug, Clone)]
pub struct FlatNode {
    pub node_id: String,
    pub depth: u32,
    pub task: String,
    pub status: String,
    pub node_type: Option<String>,
    pub cost_usd: f64,
    pub tokens: u32,
    pub errors: u32,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub children: Vec<FlatNode>,
}

impl FlatNode {
    /// Recursively collect all nodes in depth-first order.
    pub fn flatten<'a>(node: &'a Self, out: &mut Vec<&'a Self>) {
        out.push(node);
        for child in &node.children {
            Self::flatten(child, out);
        }
    }
}
