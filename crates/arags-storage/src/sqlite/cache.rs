use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use super::conn::Storage;

impl Storage {
    /// Get a cached result by task hash and project.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_cached_result(&self, task_hash: &str, project: &str) -> Result<Option<String>> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.query_row(
            "SELECT result FROM result_cache WHERE task_hash = ?1 AND project = ?2",
            params![task_hash, project],
            |row| row.get(0),
        )
        .optional()
        .context("failed to get cached result")
    }

    /// Insert or replace a cached result.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub fn put_cached_result(&self, task_hash: &str, project: &str, result: &str) -> Result<()> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.execute(
            "INSERT INTO result_cache (task_hash, project, result) VALUES (?1, ?2, ?3)
             ON CONFLICT(task_hash, project) DO UPDATE SET result = excluded.result, hit_count = hit_count + 1",
            params![task_hash, project, result],
        )
        .context("failed to insert cached result")?;

        Ok(())
    }

    /// Delete all cached results for a project.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete fails.
    pub fn invalidate_project_cache(&self, project: &str) -> Result<usize> {
        let conn = self.conn();
        let conn = conn.lock();

        let rows = conn
            .execute(
                "DELETE FROM result_cache WHERE project = ?1",
                params![project],
            )
            .context("failed to invalidate project cache")?;

        Ok(rows)
    }
}

/// Invalidate cached results via the storage connection.
///
/// When `project` is `Some`, only that project's entries are removed; when
/// `None`, the entire `result_cache` is purged. Returns the number of rows
/// deleted. Intended to be called only by an admin — the gRPC handler
/// (`arags_server::grpc::query_cache`) enforces the role gate; surfacing
/// `invalidated_by` for audit is the caller's responsibility.
///
/// # Errors
///
/// Returns an error if the delete fails.
pub fn invalidate_cache(storage: &Storage, project: Option<&str>) -> Result<u64> {
    let project = project.map(str::to_string);
    let project_str: &str = match &project {
        Some(s) => s.as_str(),
        None => "",
    };
    let has_project = project.is_some();
    let conn = storage.conn();
    let conn = conn.lock();
    let sql = if has_project {
        "DELETE FROM result_cache WHERE project = ?1"
    } else {
        "DELETE FROM result_cache"
    };
    let params: &[&dyn rusqlite::ToSql] = if has_project { &[&project_str] } else { &[] };
    let rows = conn
        .execute(sql, params)
        .context("failed to invalidate result cache")?;
    Ok(rows as u64)
}
