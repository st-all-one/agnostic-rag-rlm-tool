#![allow(
    unsafe_code,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::needless_borrow,
    clippy::unnecessary_literal_bound,
    clippy::float_cmp,
    clippy::duration_suboptimal_units,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use std::sync::Arc;

use arlm_cli::output::LiveTree;
use arlm_core::{NodeStatus, RlmEvent};

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
        status: NodeStatus::Completed,
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
        status: NodeStatus::Failed,
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
