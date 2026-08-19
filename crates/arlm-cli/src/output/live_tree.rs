use std::collections::HashMap;

use arlm_core::RlmEvent;

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
    nodes: HashMap<String, LiveNode>,
    root_id: Option<String>,
}

impl LiveTree {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            root_id: None,
        }
    }

    pub fn apply(&mut self, event: &RlmEvent) {
        match event {
            RlmEvent::RunStart {
                run_id,
                task,
                started_at_ms: _,
                ..
            } => {
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
                        arlm_core::NodeStatus::Completed | arlm_core::NodeStatus::Cached => {
                            "complete".to_string()
                        }
                        arlm_core::NodeStatus::Failed => "failed".to_string(),
                        arlm_core::NodeStatus::Cancelled | arlm_core::NodeStatus::Skipped => {
                            "cancelled".to_string()
                        }
                        arlm_core::NodeStatus::Running => "running".to_string(),
                    };
                    node.duration_ms = *duration_ms;
                    node.cost = *cost;
                }
            }
            RlmEvent::CacheHit { .. } | RlmEvent::RunEnd { .. } => {}
        }
    }

    #[must_use]
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        if let Some(root_id) = &self.root_id {
            self.render_node(root_id, &mut lines, "", true, true);
        }
        lines.join("\n")
    }

    fn render_node(
        &self,
        id: &str,
        lines: &mut Vec<String>,
        prefix: &str,
        is_last: bool,
        is_root: bool,
    ) {
        if let Some(node) = self.nodes.get(id) {
            let icon = match node.status.as_str() {
                "complete" => "\u{2713}",
                "planning" | "running" => "\u{2026}",
                "solving" => "\u{00b7}",
                "failed" => "\u{2717}",
                "cancelled" => "\u{2298}",
                _ => "?",
            };

            let duration_str = if node.duration_ms > 0 {
                format!(" {}ms", node.duration_ms)
            } else {
                String::new()
            };

            let cost_str = if node.cost > 0.0 {
                format!(" ${:.4}", node.cost)
            } else {
                String::new()
            };

            let task_display = if node.task.len() > 60 {
                format!("{}...", &node.task[..57])
            } else {
                node.task.clone()
            };

            let connector = if is_root {
                ""
            } else if is_last {
                "\u{2514}\u{2500} "
            } else {
                "\u{251c}\u{2500} "
            };

            lines.push(format!(
                "{prefix}{connector}{icon} {id} (d{depth}) {task}{dur}{cost}",
                prefix = prefix,
                connector = connector,
                icon = icon,
                id = node.id,
                depth = node.depth,
                task = task_display,
                dur = duration_str,
                cost = cost_str,
            ));

            let child_prefix = if is_root {
                String::new()
            } else if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}\u{2502}   ")
            };

            let children: Vec<&str> = self
                .nodes
                .values()
                .filter(|n| n.parent.as_deref() == Some(id))
                .map(|n| n.id.as_str())
                .collect();

            for (i, child_id) in children.iter().enumerate() {
                self.render_node(
                    child_id,
                    lines,
                    &child_prefix,
                    i == children.len() - 1,
                    false,
                );
            }
        }
    }
}

impl Default for LiveTree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_new_tree_is_empty() {
        let tree = LiveTree::new();
        assert!(tree.render().is_empty());
    }

    #[test]
    fn test_apply_run_start() {
        let mut tree = LiveTree::new();
        tree.apply(&RlmEvent::RunStart {
            run_id: Arc::from("run-1"),
            task: "build a web app".to_string(),
            backend: "openai".to_string(),
            mode: "auto".to_string(),
            max_depth: 3,
            max_nodes: 50,
            max_budget: 1.0,
            started_at_ms: 1000,
        });
        let rendered = tree.render();
        assert!(rendered.contains("run-1"));
        assert!(rendered.contains("build a web app"));
        assert!(rendered.contains("\u{2026}")); // running icon
    }

    #[test]
    fn test_apply_node_start() {
        let mut tree = LiveTree::new();
        tree.apply(&RlmEvent::RunStart {
            run_id: Arc::from("run-1"),
            task: "root task".to_string(),
            backend: "openai".to_string(),
            mode: "auto".to_string(),
            max_depth: 3,
            max_nodes: 50,
            max_budget: 1.0,
            started_at_ms: 1000,
        });
        tree.apply(&RlmEvent::NodeStart {
            run_id: Arc::from("run-1"),
            node_id: "n1".to_string(),
            depth: 1,
            task: "child task".to_string(),
            parent_id: Some("run-1".to_string()),
        });
        let rendered = tree.render();
        assert!(rendered.contains("n1"));
        assert!(rendered.contains("child task"));
    }

    #[test]
    fn test_apply_node_end_completed() {
        let mut tree = LiveTree::new();
        tree.apply(&RlmEvent::RunStart {
            run_id: Arc::from("run-1"),
            task: "root".to_string(),
            backend: "openai".to_string(),
            mode: "auto".to_string(),
            max_depth: 3,
            max_nodes: 50,
            max_budget: 1.0,
            started_at_ms: 1000,
        });
        tree.apply(&RlmEvent::NodeStart {
            run_id: Arc::from("run-1"),
            node_id: "n1".to_string(),
            depth: 1,
            task: "child".to_string(),
            parent_id: Some("run-1".to_string()),
        });
        tree.apply(&RlmEvent::NodeEnd {
            run_id: Arc::from("run-1"),
            node_id: "n1".to_string(),
            status: arlm_core::NodeStatus::Completed,
            duration_ms: 150,
            cost: 0.001,
        });
        let rendered = tree.render();
        assert!(rendered.contains("\u{2713}")); // complete icon
        assert!(rendered.contains("150ms"));
        assert!(rendered.contains("$0.0010"));
    }

    #[test]
    fn test_apply_node_end_failed() {
        let mut tree = LiveTree::new();
        tree.apply(&RlmEvent::RunStart {
            run_id: Arc::from("run-1"),
            task: "root".to_string(),
            backend: "openai".to_string(),
            mode: "auto".to_string(),
            max_depth: 3,
            max_nodes: 50,
            max_budget: 1.0,
            started_at_ms: 1000,
        });
        tree.apply(&RlmEvent::NodeEnd {
            run_id: Arc::from("run-1"),
            node_id: "run-1".to_string(),
            status: arlm_core::NodeStatus::Failed,
            duration_ms: 50,
            cost: 0.0,
        });
        let rendered = tree.render();
        assert!(rendered.contains("\u{2717}")); // failed icon
    }

    #[test]
    fn test_render_tree_indentation() {
        let mut tree = LiveTree::new();
        tree.apply(&RlmEvent::RunStart {
            run_id: Arc::from("run-1"),
            task: "root".to_string(),
            backend: "openai".to_string(),
            mode: "auto".to_string(),
            max_depth: 3,
            max_nodes: 50,
            max_budget: 1.0,
            started_at_ms: 1000,
        });
        tree.apply(&RlmEvent::NodeStart {
            run_id: Arc::from("run-1"),
            node_id: "n1".to_string(),
            depth: 1,
            task: "child 1".to_string(),
            parent_id: Some("run-1".to_string()),
        });
        tree.apply(&RlmEvent::NodeStart {
            run_id: Arc::from("run-1"),
            node_id: "n2".to_string(),
            depth: 1,
            task: "child 2".to_string(),
            parent_id: Some("run-1".to_string()),
        });
        tree.apply(&RlmEvent::NodeStart {
            run_id: Arc::from("run-1"),
            node_id: "n3".to_string(),
            depth: 2,
            task: "grandchild".to_string(),
            parent_id: Some("n1".to_string()),
        });
        let rendered = tree.render();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 4);
        // First line (root) has no tree connector prefix
        assert!(lines[0].contains("run-1"));
        // Children have tree connectors
        assert!(lines[1].contains("\u{251c}\u{2500}") || lines[1].contains("\u{2514}\u{2500}"));
    }

    #[test]
    fn test_cost_update() {
        let mut tree = LiveTree::new();
        tree.apply(&RlmEvent::RunStart {
            run_id: Arc::from("run-1"),
            task: "root".to_string(),
            backend: "openai".to_string(),
            mode: "auto".to_string(),
            max_depth: 3,
            max_nodes: 50,
            max_budget: 1.0,
            started_at_ms: 1000,
        });
        tree.apply(&RlmEvent::CostUpdate {
            run_id: Arc::from("run-1"),
            spent: 0.5,
            budget: 1.0,
        });
        let rendered = tree.render();
        assert!(rendered.contains("$0.5000"));
    }
}
