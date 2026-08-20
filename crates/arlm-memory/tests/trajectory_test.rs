#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use arlm_memory::trajectory::*;
use arlm_storage::Storage;
use tempfile::TempDir;

fn setup() -> (TrajectoryEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (TrajectoryEngine::new(storage).unwrap(), tmp)
}

fn make_root() -> DecompositionNode {
    DecompositionNode {
        description: "root task".to_string(),
        status: "completed".to_string(),
        children: vec![
            DecompositionNode {
                description: "subtask 1".to_string(),
                status: "completed".to_string(),
                children: Vec::new(),
            },
            DecompositionNode {
                description: "subtask 2".to_string(),
                status: "completed".to_string(),
                children: Vec::new(),
            },
        ],
    }
}

#[test]
fn test_store_and_find() {
    let (eng, _tmp) = setup();
    let root = make_root();

    let id = eng
        .store("my-project", "fix the bug", &root, Some(0.5))
        .unwrap();
    assert!(id > 0);

    let found = eng.find_by_hash("fix the bug", "my-project").unwrap();
    assert!(found.is_some());

    let t = found.unwrap();
    assert_eq!(t.task, "fix the bug");
    assert_eq!(t.root.children.len(), 2);
}

#[test]
fn test_find_by_hash_not_found() {
    let (eng, _tmp) = setup();
    let result = eng.find_by_hash("nonexistent", "proj").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_find_similar() {
    let (eng, _tmp) = setup();
    let root = make_root();

    eng.store("proj", "fix login bug", &root, None).unwrap();
    eng.store("proj", "fix auth bug", &root, None).unwrap();
    eng.store("proj", "add tests", &root, None).unwrap();

    let opts = FindSimilarOptions::default();
    let similar = eng.find_similar("fix", "proj", &opts).unwrap();
    assert_eq!(similar.len(), 2);
}

#[test]
fn test_replay_strategy() {
    let (eng, _tmp) = setup();
    let root = make_root();

    eng.store("proj", "deploy app", &root, None).unwrap();

    let steps = eng.replay_strategy("deploy app", "proj").unwrap();
    assert!(steps.is_some());

    let steps = steps.unwrap();
    assert_eq!(steps.len(), 3);
    assert!(steps.contains(&"root task".to_string()));
    assert!(steps.contains(&"subtask 1".to_string()));
}

#[test]
fn test_list() {
    let (eng, _tmp) = setup();
    let root = DecompositionNode {
        description: "t".to_string(),
        status: "done".to_string(),
        children: Vec::new(),
    };

    eng.store("proj", "task1", &root, None).unwrap();
    eng.store("proj", "task2", &root, None).unwrap();

    let list = eng.list("proj", 10).unwrap();
    assert_eq!(list.len(), 2);
}

#[test]
fn test_flatten_decomposition() {
    let root = make_root();
    let steps = flatten_decomposition(&root);
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0], "root task");
    assert_eq!(steps[1], "subtask 1");
    assert_eq!(steps[2], "subtask 2");
}

#[test]
fn test_compute_task_hash_deterministic() {
    let h1 = compute_task_hash("test task");
    let h2 = compute_task_hash("test task");
    let h3 = compute_task_hash("other task");
    assert_eq!(h1, h2);
    assert_ne!(h1, h3);
}
