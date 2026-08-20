//! Run persistence and run-status mapping.

use anyhow::{Context, Result};
use arlm_proto::proto::RunStatus as ProtoRunStatus;
use arlm_storage::Storage;
use rusqlite::params;

use super::RunRow;

const RUN_COLUMNS: &str = "id, project, task, backend, model, status, partial_answer, started_at, finished_at, duration_ms, total_tokens, total_cost, nodes_visited, max_depth";

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: row.get(0)?,
        project: row.get(1)?,
        task: row.get(2)?,
        backend: row.get(3)?,
        model: row.get(4)?,
        status: row.get(5)?,
        answer: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        duration_ms: row.get(9)?,
        total_tokens: row.get(10)?,
        total_cost: row.get(11)?,
        nodes_visited: row.get::<_, Option<i64>>(12)?.unwrap_or(0),
        max_depth: row.get::<_, Option<i64>>(13)?.unwrap_or(0),
    })
}

/// Map a persisted run status string to the proto enum.
#[must_use]
pub fn proto_run_status(status: &str) -> ProtoRunStatus {
    match status {
        "running" => ProtoRunStatus::StatusRunning,
        "completed" => ProtoRunStatus::StatusCompleted,
        "failed" => ProtoRunStatus::StatusFailed,
        "cancelled" | "canceled" => ProtoRunStatus::StatusCancelled,
        _ => ProtoRunStatus::StatusPending,
    }
}

/// Map the proto run status enum to a persisted status string.
#[must_use]
pub fn db_run_status(status: ProtoRunStatus) -> &'static str {
    match status {
        ProtoRunStatus::StatusRunning => "running",
        ProtoRunStatus::StatusCompleted => "completed",
        ProtoRunStatus::StatusFailed => "failed",
        ProtoRunStatus::StatusCancelled => "cancelled",
        ProtoRunStatus::StatusPending => "pending",
    }
}

/// Insert a run record in its initial (`running`) state.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_run(storage: &Storage, run: &RunRow) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO runs (id, project, task, backend, model, status, started_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                run.id,
                run.project,
                run.task,
                run.backend,
                run.model,
                run.status,
                run.started_at,
            ],
        )?;
        Ok(())
    })
    .context("failed to insert run")
}

/// Fetch a run by id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_run(storage: &Storage, run_id: &str) -> Result<Option<RunRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?1"))
            .context("failed to prepare get_run query")?;
        let mut rows = stmt.query_map(params![run_id], row_to_run)?;
        Ok(rows.next().transpose()?)
    })
    .context("failed to get run")
}

/// Cancel a running run.
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn cancel_run(storage: &Storage, run_id: &str) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "UPDATE runs SET status = 'cancelled', finished_at = ?1 WHERE id = ?2",
            params![chrono::Utc::now().timestamp(), run_id],
        )?;
        Ok(())
    })
    .context("failed to cancel run")
}

/// Mark a run as completed with its final result.
///
/// # Errors
///
/// Returns an error if the update fails.
#[allow(clippy::too_many_arguments)]
pub fn complete_run(
    storage: &Storage,
    run_id: &str,
    answer: &str,
    duration_ms: u64,
    nodes_visited: u32,
    max_depth: u32,
    total_tokens: u64,
    total_cost: f64,
) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "UPDATE runs SET status = 'completed', partial_answer = ?1, finished_at = ?2, \
             duration_ms = ?3, nodes_visited = ?4, max_depth = ?5, total_tokens = ?6, \
             total_cost = ?7 WHERE id = ?8",
            params![
                answer,
                chrono::Utc::now().timestamp(),
                u64::try_from(duration_ms).unwrap_or(u64::MAX),
                u64::try_from(nodes_visited).unwrap_or(u64::MAX),
                u64::try_from(max_depth).unwrap_or(u64::MAX),
                u64::try_from(total_tokens).unwrap_or(u64::MAX),
                total_cost,
                run_id,
            ],
        )?;
        Ok(())
    })
    .context("failed to complete run")
}

/// Mark a run as failed with an error message.
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn fail_run(storage: &Storage, run_id: &str, error: &str) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "UPDATE runs SET status = 'failed', partial_answer = ?1, finished_at = ?2 \
             WHERE id = ?3",
            params![error, chrono::Utc::now().timestamp(), run_id,],
        )?;
        Ok(())
    })
    .context("failed to fail run")
}

/// Count runs currently in the `running` state.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn count_active_runs(storage: &Storage) -> Result<u32> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        Ok(u32::try_from(
            conn.query_row(
                "SELECT COUNT(*) FROM runs WHERE status = 'running'",
                [],
                |row| row.get::<_, i64>(0),
            )?
            .max(0),
        )
        .unwrap_or(u32::MAX))
    })
    .context("failed to count active runs")
}
