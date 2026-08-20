//! Session persistence: `sessions` rows and `session_history` turns.

use anyhow::{Context, Result};
use arlm_storage::Storage;
use rusqlite::params;

use super::SessionRow;

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        project: row.get(1)?,
        title: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

/// Insert a session for a project.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_session(storage: &Storage, id: &str, project: &str, title: &str) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let now = chrono::Utc::now().timestamp();

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, project_name, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, project, title, now],
        )?;
        Ok(())
    })
    .context("failed to insert session")
}

/// List sessions for a project, newest first.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn list_sessions(storage: &Storage, project: &str) -> Result<Vec<SessionRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, project_name, title, created_at, updated_at \
                 FROM sessions WHERE project_name = ?1 ORDER BY created_at DESC",
            )
            .context("failed to prepare list_sessions query")?;
        let rows = stmt
            .query_map(params![project], row_to_session)?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(rows)
    })
    .context("failed to list sessions")
}

/// Get a single session by id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_session(storage: &Storage, id: &str) -> Result<Option<SessionRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, project_name, title, created_at, updated_at \
                 FROM sessions WHERE id = ?1",
            )
            .context("failed to prepare get_session query")?;
        let mut rows = stmt.query_map(params![id], row_to_session)?;
        Ok(rows.next().transpose()?)
    })
    .context("failed to get session")
}

/// Count the number of turns stored for a session.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn count_session_turns(storage: &Storage, session_id: &str) -> Result<i64> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM session_history WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?)
    })
    .context("failed to count session turns")
}

/// Persist a session turn (query/result pair) and bump `updated_at`.
///
/// # Errors
///
/// Returns an error if the insert or update fails.
pub fn insert_session_turn(
    storage: &Storage,
    session_id: &str,
    query: &str,
    result: &str,
) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let now = chrono::Utc::now().timestamp();

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO session_history (session_id, query, result) VALUES (?1, ?2, ?3)",
            params![session_id, query, result],
        )?;
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        Ok(())
    })
    .context("failed to insert session turn")
}
