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

use arags_storage::sqlite::schema::{MIGRATION_COUNT, run_migrations};
use rusqlite::Connection;
use tempfile::TempDir;

#[test]
fn test_migrations_run() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = Connection::open(&db_path).unwrap();

    run_migrations(&conn).unwrap();

    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(std::result::Result::ok)
        .collect();

    for expected in [
        "chunks",
        "buffers",
        "tasks",
        "findings",
        "history",
        "patterns",
        "schema_version",
        "runs",
        "run_model_usage",
        "node_calls",
        "trajectories",
        "result_cache",
        "events",
    ] {
        assert!(
            tables.contains(&expected.to_string()),
            "missing table {expected}"
        );
    }
}

#[test]
fn test_migrations_are_idempotent() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = Connection::open(&db_path).unwrap();

    run_migrations(&conn).unwrap();
    run_migrations(&conn).unwrap();

    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .unwrap();
    // Idempotent: a second run must not add rows or change the version.
    assert_eq!(version as usize, MIGRATION_COUNT);
}
