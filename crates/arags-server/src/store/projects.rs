//! Project (buffer) persistence.

use anyhow::{Context, Result};
use arags_storage::Storage;
use rusqlite::{OptionalExtension, params};

use super::ProjectRow;

const PROJECT_COLUMNS: &str = "id, uuid, name, path, total_chunks, total_files, created_at";

fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRow> {
    Ok(ProjectRow {
        id: row.get(0)?,
        uuid: row.get(1)?,
        name: row.get(2)?,
        path: row.get(3)?,
        total_chunks: row.get(4)?,
        total_files: row.get(5)?,
        created_at: row.get(6)?,
    })
}

/// Insert a project (buffer) and return its numeric id.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_project(storage: &Storage, name: &str, path: &str) -> Result<i64> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let uuid = uuid::Uuid::now_v7().to_string();

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO buffers (name, path, uuid) VALUES (?1, ?2, ?3)",
            params![name, path, uuid],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .context("failed to insert project")
}

/// Ensure a project (buffer) exists, creating it if necessary, and return its
/// numeric id.
///
/// Safe to call concurrently: `INSERT OR IGNORE` dedupes on the unique `name`,
/// and the id is read back in the *same* connection. Performing both steps
/// under a single pooled connection matters under concurrent index streams —
/// acquiring two connections sequentially would deadlock against a small pool
/// when the number of parallel streams exceeds `pool_size`.
///
/// # Errors
///
/// Returns an error if the insert or lookup fails.
pub fn ensure_project(storage: &Storage, name: &str, path: &str) -> Result<i64> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let uuid = uuid::Uuid::now_v7().to_string();

    conn.execute(|conn| {
        conn.execute(
            "INSERT OR IGNORE INTO buffers (name, path, uuid, total_chunks, total_files) \
             VALUES (?1, ?2, ?3, 0, 0)",
            params![name, path, uuid],
        )?;
        let id = conn.query_row(
            "SELECT id FROM buffers WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;
        Ok(id)
    })
    .context("failed to ensure project")
}

/// Look up a project by name.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_project_by_name(storage: &Storage, name: &str) -> Result<Option<ProjectRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM buffers WHERE name = ?1"
            ))
            .context("failed to prepare get_project_by_name query")?;
        let mut rows = stmt.query_map(params![name], row_to_project)?;
        Ok(rows.next().transpose()?)
    })
    .context("failed to query project by name")
}

/// Look up a project by numeric id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_project_by_id(storage: &Storage, id: i64) -> Result<Option<ProjectRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM buffers WHERE id = ?1"
            ))
            .context("failed to prepare get_project_by_id query")?;
        let mut rows = stmt.query_map(params![id], row_to_project)?;
        Ok(rows.next().transpose()?)
    })
    .context("failed to query project by id")
}

/// Look up a project by uuid.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_project_by_uuid(storage: &Storage, uuid: &str) -> Result<Option<ProjectRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM buffers WHERE uuid = ?1"
            ))
            .context("failed to prepare get_project_by_uuid query")?;
        let mut rows = stmt.query_map(params![uuid], row_to_project)?;
        Ok(rows.next().transpose()?)
    })
    .context("failed to query project by uuid")
}

/// List every project ordered by name.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn list_projects(storage: &Storage) -> Result<Vec<ProjectRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM buffers ORDER BY name"
            ))
            .context("failed to prepare list_projects query")?;
        let rows = stmt
            .query_map([], row_to_project)?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(rows)
    })
    .context("failed to list projects")
}

/// Buffer id for a project name or uuid.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn buffer_id_for_project(storage: &Storage, project: &str) -> Result<Option<i64>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        Ok(conn
            .query_row(
                "SELECT id FROM buffers WHERE name = ?1 OR uuid = ?1",
                params![project],
                |row| row.get(0),
            )
            .optional()?)
    })
    .context("failed to find buffer for project")
}
