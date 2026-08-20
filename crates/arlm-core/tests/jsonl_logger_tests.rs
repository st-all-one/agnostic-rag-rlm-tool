#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use arlm_core::events::{EventBus, RlmEvent};
use arlm_core::jsonl_logger::{JsonlEventLogger, files, spawn_event_writer};
use std::sync::Arc;

fn sample_events() -> Vec<RlmEvent> {
    vec![
        RlmEvent::RunStart {
            run_id: Arc::from("run-test"),
            task: "solve the problem".to_string(),
            backend: "mock".to_string(),
            mode: "auto".to_string(),
            max_depth: 3,
            max_nodes: 10,
            max_budget: 1.0,
            started_at_ms: 1_700_000_000_000,
        },
        RlmEvent::CostUpdate {
            run_id: Arc::from("run-test"),
            spent: 0.5,
            budget: 1.0,
        },
        RlmEvent::RunEnd {
            run_id: Arc::from("run-test"),
            duration_ms: 100,
            nodes_visited: 5,
        },
    ]
}

#[test]
fn test_log_and_replay_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let logger = JsonlEventLogger::new("run-1", dir.path()).expect("logger");
    let events = sample_events();

    for event in &events {
        logger.log(event).expect("log");
    }

    let replayed = JsonlEventLogger::replay(logger.log_path()).expect("replay");
    assert_eq!(replayed.len(), events.len());
    for (original, replay) in events.iter().zip(&replayed) {
        let a = serde_json::to_string(original).expect("serialize original");
        let b = serde_json::to_string(replay).expect("serialize replayed");
        assert_eq!(a, b);
    }
}

#[test]
fn test_new_creates_directory() {
    let base = tempfile::tempdir().expect("tempdir");
    let nested = base.path().join("a/b/c");
    let logger = JsonlEventLogger::new("run-1", &nested).expect("logger");
    assert!(logger.log_path().is_file());
    assert_eq!(logger.log_dir(), &nested);
    assert_eq!(logger.run_id(), Some("run-1"));
}

#[test]
fn test_files_finds_event_logs() {
    let dir = tempfile::tempdir().expect("tempdir");
    JsonlEventLogger::new("run-1", dir.path()).expect("logger");
    JsonlEventLogger::new("run-2", dir.path()).expect("logger");

    let mut log_files = files(dir.path());
    log_files.sort();
    assert_eq!(log_files.len(), 2);

    let names: Vec<String> = log_files
        .iter()
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect();
    assert_eq!(
        names,
        vec!["run_run-1.events.jsonl", "run_run-2.events.jsonl"]
    );
}

#[test]
fn test_files_missing_dir_is_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(files(&dir.path().join("missing")).is_empty());
}

#[test]
fn test_replay_ignores_empty_lines() {
    let dir = tempfile::tempdir().expect("tempdir");
    let logger = JsonlEventLogger::new("run-1", dir.path()).expect("logger");
    logger.log(&sample_events()[0]).expect("log");

    std::fs::write(logger.log_path(), b"\n").expect("truncate");
    let replayed = JsonlEventLogger::replay(logger.log_path()).expect("replay");
    assert!(replayed.is_empty());
}

#[tokio::test]
async fn test_spawn_event_writer_writes_all_events() {
    let dir = tempfile::tempdir().expect("tempdir");
    let logger = Arc::new(JsonlEventLogger::new("run-2", dir.path()).expect("logger"));
    let bus = EventBus::new();
    let handle = spawn_event_writer(bus.subscribe(), logger);
    let mut check_rx = bus.subscribe();

    let events = sample_events();
    let count = events.len();
    for event in &events {
        bus.emit(event.clone());
    }

    for _ in 0..count {
        check_rx.recv().await.expect("should receive event");
    }
    drop(check_rx);
    drop(bus);
    handle.await.expect("writer task should complete");

    let replayed = JsonlEventLogger::replay(dir.path().join("run_run-2.events.jsonl").as_path())
        .expect("replay");
    assert_eq!(replayed.len(), count);
}
