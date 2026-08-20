//! Integration tests for the typed store layer (`arlm_server::store`).
//!
//! Storage is opened in single-connection mode (SQLite in a temp dir) so the
//! tests are fast and hermetic; the same functions are used by the server's
//! pooled mode.

use arlm_server::store;
use arlm_storage::Storage;
use tempfile::TempDir;

fn setup() -> (Storage, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    // Single-header DB at a fixed path.
    let storage = Storage::open(dir.path()).expect("open storage");
    (storage, dir)
}

// ── Projects ───────────────────────────────────────────────────────────────

#[test]
fn test_insert_and_get_project_by_name() {
    let (storage, _dir) = setup();
    let id = store::insert_project(&storage, "my-project", "/path/to/project").unwrap();
    assert!(id > 0);

    let row = store::get_project_by_name(&storage, "my-project")
        .unwrap()
        .expect("project exists");
    assert_eq!(row.id, id);
    assert_eq!(row.path, "/path/to/project");
    assert!(row.uuid.is_some());
}

#[test]
fn test_get_project_by_uuid() {
    let (storage, _dir) = setup();
    store::insert_project(&storage, "p1", "/p1").unwrap();

    let by_name = store::get_project_by_name(&storage, "p1").unwrap().unwrap();
    let by_uuid = store::get_project_by_uuid(&storage, by_name.uuid.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(by_uuid.id, by_name.id);
}

#[test]
fn test_list_projects() {
    let (storage, _dir) = setup();
    for i in 0..3 {
        store::insert_project(&storage, &format!("proj-{i}"), &format!("/p{i}")).unwrap();
    }
    let projects = store::list_projects(&storage).unwrap();
    assert_eq!(projects.len(), 3);
}

// ── Sessions ───────────────────────────────────────────────────────────────

#[test]
fn test_session_crud_and_turns() {
    let (storage, _dir) = setup();
    store::insert_project(&storage, "proj", "/p").unwrap();

    let session_id = "sess-1";
    store::insert_session(&storage, session_id, "proj", "Hello").unwrap();

    let row = store::get_session(&storage, session_id).unwrap().expect("session");
    assert_eq!(row.title, "Hello");

    store::insert_session_turn(&storage, session_id, "query", "answer").unwrap();
    assert_eq!(store::count_session_turns(&storage, session_id).unwrap(), 1);

    let sessions = store::list_sessions(&storage, "proj").unwrap();
    assert_eq!(sessions.len(), 1);
}

// ── Runs ───────────────────────────────────────────────────────────────────

#[test]
fn test_run_lifecycle() {
    let (storage, _dir) = setup();
    store::insert_project(&storage, "proj", "/p").unwrap();

    let run = store::RunRow {
        id: "run-1".to_string(),
        project: Some("proj".to_string()),
        task: "task".to_string(),
        backend: Some("openai".to_string()),
        model: Some("gpt-4".to_string()),
        status: "running".to_string(),
        answer: None,
        started_at: Some(0),
        finished_at: None,
        duration_ms: None,
        total_tokens: 0,
        total_cost: 0.0,
        nodes_visited: 0,
        max_depth: 0,
    };
    store::insert_run(&storage, &run).unwrap();

    let fetched = store::get_run(&storage, "run-1").unwrap().expect("run");
    assert_eq!(fetched.status, "running");

    store::complete_run(&storage, "run-1", "done", 100, 5, 2, 1000, 0.01).unwrap();
    let completed = store::get_run(&storage, "run-1").unwrap().unwrap();
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.answer.as_deref(), Some("done"));
    assert!(completed.duration_ms.is_some());

    assert_eq!(store::count_active_runs(&storage).unwrap(), 0);
}

#[test]
fn test_cancel_run_and_proto_status() {
    let (storage, _dir) = setup();
    store::insert_run(
        &storage,
        &store::RunRow {
            id: "run-2".to_string(),
            project: None,
            task: "t".to_string(),
            backend: None,
            model: None,
            status: "running".to_string(),
            answer: None,
            started_at: Some(0),
            finished_at: None,
            duration_ms: None,
            total_tokens: 0,
            total_cost: 0.0,
            nodes_visited: 0,
            max_depth: 0,
        },
    )
    .unwrap();

    assert_eq!(store::count_active_runs(&storage).unwrap(), 1);
    store::cancel_run(&storage, "run-2").unwrap();
    assert_eq!(store::count_active_runs(&storage).unwrap(), 0);

    assert_eq!(
        store::proto_run_status("completed"),
        arlm_proto::proto::RunStatus::StatusCompleted
    );
    assert_eq!(store::db_run_status(arlm_proto::proto::RunStatus::StatusRunning), "running");
}