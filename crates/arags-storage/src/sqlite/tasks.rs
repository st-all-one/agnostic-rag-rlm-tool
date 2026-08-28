use anyhow::{Context, Result};
use rusqlite::params;
use tracing::info;

use super::conn::Storage;

/// Task for dispatch.
#[derive(Debug, Clone)]
pub struct Task {
    pub id: i64,
    pub buffer_id: i64,
    pub chunk_id: Option<i64>,
    pub status: String,
    pub assigned_to: Option<String>,
    pub payload: Option<String>,
    pub result: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

impl Storage {
    /// Insert a new task and return its ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub fn insert_task(
        &self,
        buffer_id: i64,
        chunk_id: Option<i64>,
        payload: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn();
        let conn = conn.lock();
        let start = std::time::Instant::now();

        let id = conn
            .execute(
                "INSERT INTO tasks (buffer_id, chunk_id, payload) VALUES (?1, ?2, ?3)",
                params![buffer_id, chunk_id, payload],
            )
            .context("failed to insert task")?;

        let task_id = i64::try_from(id).context("task id overflow")?;
        info!(task_id, buffer_id, duration_ms = %start.elapsed().as_millis(), "inserted task");

        Ok(task_id)
    }

    /// Get pending tasks for a buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_pending_tasks(&self, buffer_id: i64, limit: i64) -> Result<Vec<Task>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT id, buffer_id, chunk_id, status, assigned_to, payload, result, created_at, started_at, finished_at
                 FROM tasks WHERE buffer_id = ?1 AND status = 'pending' ORDER BY id LIMIT ?2",
            )
            .context("failed to prepare get_pending_tasks query")?;

        let rows = stmt
            .query_map(params![buffer_id, limit], |row| {
                Ok(Task {
                    id: row.get(0)?,
                    buffer_id: row.get(1)?,
                    chunk_id: row.get(2)?,
                    status: row.get(3)?,
                    assigned_to: row.get(4)?,
                    payload: row.get(5)?,
                    result: row.get(6)?,
                    created_at: row.get(7)?,
                    started_at: row.get(8)?,
                    finished_at: row.get(9)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }

    /// Update task status.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn update_task_status(
        &self,
        task_id: i64,
        status: &str,
        assigned_to: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.execute(
            "UPDATE tasks SET status = ?1, assigned_to = ?2, started_at = CASE WHEN ?1 = 'running' THEN unixepoch() ELSE started_at END WHERE id = ?3",
            params![status, assigned_to, task_id],
        )
        .context("failed to update task status")?;

        Ok(())
    }

    /// Complete a task with result.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn complete_task(&self, task_id: i64, result: &str) -> Result<()> {
        let conn = self.conn();
        let conn = conn.lock();
        let start = std::time::Instant::now();

        conn.execute(
            "UPDATE tasks SET status = 'done', result = ?1, finished_at = unixepoch() WHERE id = ?2",
            params![result, task_id],
        )
        .context("failed to complete task")?;

        info!(task_id, duration_ms = %start.elapsed().as_millis(), "completed task");

        Ok(())
    }
}
