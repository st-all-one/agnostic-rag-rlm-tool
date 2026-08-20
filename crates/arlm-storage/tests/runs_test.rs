#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::float_cmp
)]

use arlm_storage::Storage;
use arlm_storage::sqlite::nodes::FlatNode;
use tempfile::TempDir;

fn setup_storage() -> (Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (storage, tmp)
}

#[test]
fn test_insert_and_get_run() {
    let (storage, _tmp) = setup_storage();
    storage
        .insert_run(
            "run-001",
            "test task",
            "openai",
            "auto",
            "completed",
            "arlm",
            1000,
            500,
            0.05,
            150,
            3,
            2,
            5,
            None,
            None,
            None,
        )
        .unwrap();
    let run = storage.get_run("run-001").unwrap().unwrap();
    assert_eq!(run.id, "run-001");
    assert_eq!(run.task, "test task");
    assert_eq!(run.backend.as_deref(), Some("openai"));
    assert!((run.total_cost - 0.05).abs() < f64::EPSILON);
}

#[test]
fn test_list_runs() {
    let (storage, _tmp) = setup_storage();
    for i in 0..3 {
        storage
            .insert_run(
                &format!("run-{i}"),
                &format!("task {i}"),
                "openai",
                "auto",
                "completed",
                "arlm",
                1000 + i * 1000,
                500,
                0.01,
                100,
                1,
                1,
                1,
                None,
                None,
                None,
            )
            .unwrap();
    }
    let runs = storage.list_runs(10).unwrap();
    assert_eq!(runs.len(), 3);
}

#[test]
fn test_run_cost() {
    let (storage, _tmp) = setup_storage();
    let child = FlatNode {
        node_id: "c1".to_string(),
        depth: 1,
        task: "child".to_string(),
        status: "completed".to_string(),
        node_type: None,
        cost_usd: 0.04,
        tokens: 50,
        errors: 0,
        started_at_ms: 1000,
        finished_at_ms: Some(1500),
        result: None,
        error: None,
        children: vec![],
    };
    let root = FlatNode {
        node_id: "n1".to_string(),
        depth: 0,
        task: "root".to_string(),
        status: "completed".to_string(),
        node_type: None,
        cost_usd: 0.06,
        tokens: 100,
        errors: 0,
        started_at_ms: 1000,
        finished_at_ms: Some(2000),
        result: None,
        error: None,
        children: vec![child],
    };
    storage
        .insert_run(
            "run-001",
            "test",
            "openai",
            "auto",
            "completed",
            "arlm",
            1000,
            1000,
            0.10,
            150,
            2,
            1,
            2,
            None,
            None,
            Some(&root),
        )
        .unwrap();
    let cost = storage.run_cost("run-001").unwrap();
    assert!((cost - 0.10).abs() < f64::EPSILON);
}

#[test]
fn test_total_cost() {
    let (storage, _tmp) = setup_storage();
    storage
        .insert_run(
            "run-001",
            "test",
            "openai",
            "auto",
            "completed",
            "arlm",
            1000,
            500,
            0.05,
            100,
            1,
            1,
            1,
            None,
            None,
            None,
        )
        .unwrap();
    storage
        .insert_run(
            "run-002",
            "test",
            "openai",
            "auto",
            "completed",
            "arlm",
            2000,
            500,
            0.07,
            100,
            1,
            1,
            1,
            None,
            None,
            None,
        )
        .unwrap();
    let total = storage.total_cost().unwrap();
    assert!((total - 0.12).abs() < f64::EPSILON);
}

#[test]
fn test_insert_trajectory() {
    let (storage, _tmp) = setup_storage();
    let id = storage
        .insert_trajectory("my-project", None, "{}", "test task", None, 0.05)
        .unwrap();
    assert!(id > 0);
}

#[test]
fn test_insert_run_with_flat_node_tree() {
    let (storage, _tmp) = setup_storage();
    let child = FlatNode {
        node_id: "c1".to_string(),
        depth: 1,
        task: "child task".to_string(),
        status: "completed".to_string(),
        node_type: Some("solve".to_string()),
        cost_usd: 0.02,
        tokens: 50,
        errors: 0,
        started_at_ms: 1000,
        finished_at_ms: Some(1500),
        result: Some("child result".to_string()),
        error: None,
        children: vec![],
    };
    let root = FlatNode {
        node_id: "n1".to_string(),
        depth: 0,
        task: "root task".to_string(),
        status: "completed".to_string(),
        node_type: Some("decompose".to_string()),
        cost_usd: 0.03,
        tokens: 100,
        errors: 0,
        started_at_ms: 1000,
        finished_at_ms: Some(2000),
        result: Some("root result".to_string()),
        error: None,
        children: vec![child],
    };
    storage
        .insert_run(
            "run-001",
            "test task",
            "openai",
            "auto",
            "completed",
            "arlm",
            1000,
            1000,
            0.05,
            150,
            2,
            1,
            2,
            None,
            None,
            Some(&root),
        )
        .unwrap();
    let run = storage.get_run("run-001").unwrap().unwrap();
    assert_eq!(run.total_calls, 2);
}

#[test]
fn test_flat_node_flatten() {
    let child = FlatNode {
        node_id: "c1".to_string(),
        depth: 1,
        task: "child".to_string(),
        status: "completed".to_string(),
        node_type: None,
        cost_usd: 0.0,
        tokens: 0,
        errors: 0,
        started_at_ms: 0,
        finished_at_ms: None,
        result: None,
        error: None,
        children: vec![],
    };
    let root = FlatNode {
        node_id: "n1".to_string(),
        depth: 0,
        task: "root".to_string(),
        status: "completed".to_string(),
        node_type: None,
        cost_usd: 0.0,
        tokens: 0,
        errors: 0,
        started_at_ms: 0,
        finished_at_ms: None,
        result: None,
        error: None,
        children: vec![child],
    };
    let mut flat = Vec::new();
    FlatNode::flatten(&root, &mut flat);
    assert_eq!(flat.len(), 2);
    assert_eq!(flat[0].node_id, "n1");
    assert_eq!(flat[1].node_id, "c1");
}
