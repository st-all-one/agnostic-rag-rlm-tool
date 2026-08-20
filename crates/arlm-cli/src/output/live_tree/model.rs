use std::collections::HashMap;

use arlm_core::RlmEvent;
use tracing::debug;

use arlm_core::NodeStatus;

#[derive(Debug, Clone)]
pub struct LiveNode {
    pub id: String,
    pub depth: u32,
    pub task: String,
    pub status: String,
    pub parent: Option<String>,
    pub duration_ms: u64,
    pub cost: f64,
}

#[derive(Debug, Clone)]
pub struct LiveTree {
    pub(crate) nodes: HashMap<String, LiveNode>,
    pub(crate) root_id: Option<String>,
}

impl LiveTree {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            root_id: None,
        }
    }

    /// Classify an event into a short, structured category string for logging.
    #[must_use]
    fn event_kind(event: &RlmEvent) -> &'static str {
        match event {
            RlmEvent::RunStart { .. } => "run_start",
            RlmEvent::NodeStart { .. } => "node_start",
            RlmEvent::NodePlan { .. } => "node_plan",
            RlmEvent::NodeSolve { .. } => "node_solve",
            RlmEvent::NodeSynthesize { .. } => "node_synthesize",
            RlmEvent::CostUpdate { .. } => "cost_update",
            RlmEvent::NodeEnd { .. } => "node_end",
            RlmEvent::CacheHit { .. } => "cache_hit",
            RlmEvent::RunEnd { .. } => "run_end",
        }
    }

    pub fn apply(&mut self, event: &RlmEvent) {
        debug!(kind = Self::event_kind(event), "live_tree apply");
        match event {
            RlmEvent::RunStart { run_id, task, .. } => {
                let node = LiveNode {
                    id: run_id.to_string(),
                    depth: 0,
                    task: task.clone(),
                    status: "running".to_string(),
                    parent: None,
                    duration_ms: 0,
                    cost: 0.0,
                };
                self.root_id = Some(node.id.clone());
                self.nodes.insert(node.id.clone(), node);
            }
            RlmEvent::NodeStart {
                node_id,
                depth,
                task,
                parent_id,
                ..
            } => {
                let node = LiveNode {
                    id: node_id.clone(),
                    depth: *depth,
                    task: task.clone(),
                    status: "running".to_string(),
                    parent: parent_id.clone(),
                    duration_ms: 0,
                    cost: 0.0,
                };
                self.nodes.insert(node_id.clone(), node);
            }
            RlmEvent::NodePlan { node_id, .. } => {
                if let Some(node) = self.nodes.get_mut(node_id) {
                    node.status = "planning".to_string();
                }
            }
            RlmEvent::NodeSolve { node_id, .. } => {
                if let Some(node) = self.nodes.get_mut(node_id) {
                    node.status = "solving".to_string();
                }
            }
            RlmEvent::NodeSynthesize { node_id, .. } => {
                if let Some(node) = self.nodes.get_mut(node_id) {
                    node.status = "complete".to_string();
                }
            }
            RlmEvent::CostUpdate { spent, .. } => {
                if let Some(root_id) = self.root_id.clone() {
                    if let Some(root) = self.nodes.get_mut(&root_id) {
                        root.cost = *spent;
                    }
                }
            }
            RlmEvent::NodeEnd {
                node_id,
                status,
                duration_ms,
                cost,
                ..
            } => {
                if let Some(node) = self.nodes.get_mut(node_id) {
                    node.status = match status {
                        NodeStatus::Completed | NodeStatus::Cached => "complete".to_string(),
                        NodeStatus::Failed => "failed".to_string(),
                        NodeStatus::Cancelled | NodeStatus::Skipped => "cancelled".to_string(),
                        NodeStatus::Running => "running".to_string(),
                    };
                    node.duration_ms = *duration_ms;
                    node.cost = *cost;
                }
            }
            RlmEvent::CacheHit { .. } | RlmEvent::RunEnd { .. } => {}
        }
    }
}

impl Default for LiveTree {
    fn default() -> Self {
        Self::new()
    }
}
