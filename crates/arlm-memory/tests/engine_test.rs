#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use arlm_memory::engine::*;
use arlm_memory::trajectory::DecompositionNode;
use std::path::Path;
use tempfile::TempDir;

fn setup() -> (MemoryEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = MemoryEngine::open(tmp.path()).unwrap();
    (engine, tmp)
}

#[test]
fn test_create_project() {
    let (engine, _tmp) = setup();
    let info = engine
        .create_project("test-proj", Path::new("/tmp/test"))
        .unwrap();
    assert_eq!(info.name, "test-proj");
}

#[test]
fn test_list_projects_empty() {
    let (engine, _tmp) = setup();
    let projects = engine.list_projects().unwrap();
    assert!(projects.is_empty());
}

#[test]
fn test_get_project() {
    let (engine, _tmp) = setup();
    engine.create_project("my-proj", Path::new("/tmp")).unwrap();
    let proj = engine.get_project("my-proj").unwrap();
    assert!(proj.is_some());
    assert_eq!(proj.unwrap().name, "my-proj");
}

#[test]
fn test_create_session() {
    let (engine, _tmp) = setup();
    let id = engine.create_session("proj", "Analysis").unwrap();
    assert!(id.starts_with("s_"));
}

#[test]
fn test_session_context() {
    let (engine, _tmp) = setup();
    let id = engine.create_session("proj", "title").unwrap();
    let v = engine.add_session_context(&id, "context data").unwrap();
    assert_eq!(v, 1);
}

#[test]
fn test_store_and_find_trajectory() {
    let (engine, _tmp) = setup();
    let root = DecompositionNode {
        description: "root task".to_string(),
        status: "completed".to_string(),
        children: vec![],
    };
    let id = engine
        .store_trajectory("proj", "test task", &root, Some(0.05))
        .unwrap();
    assert!(id > 0);

    let similar = engine
        .find_similar_trajectories("test task", "proj")
        .unwrap();
    assert!(!similar.is_empty());
}
