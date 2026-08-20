use anyhow::{Context, Result};
use rusqlite::params;

use super::conn::Storage;

/// Finding from a subagent.
#[derive(Debug, Clone)]
pub struct Finding {
    pub id: i64,
    pub task_id: i64,
    pub chunk_id: Option<i64>,
    pub finding_type: Option<String>,
    pub content: String,
    pub confidence: Option<f64>,
    pub created_at: i64,
}

impl Storage {
    /// Insert a new finding.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub fn insert_finding(
        &self,
        task_id: i64,
        chunk_id: Option<i64>,
        finding_type: Option<&str>,
        content: &str,
        confidence: Option<f64>,
    ) -> Result<i64> {
        let conn = self.conn();
        let conn = conn.lock();

        let id = conn
            .execute(
                "INSERT INTO findings (task_id, chunk_id, finding_type, content, confidence) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![task_id, chunk_id, finding_type, content, confidence],
            )
            .context("failed to insert finding")?;

        let finding_id = i64::try_from(id).context("finding id overflow")?;
        tracing::info!(finding_id, task_id, finding_type, "inserted finding");

        Ok(finding_id)
    }

    /// Get findings for a task.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_findings_for_task(&self, task_id: i64) -> Result<Vec<Finding>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT id, task_id, chunk_id, finding_type, content, confidence, created_at
                 FROM findings WHERE task_id = ?1 ORDER BY id",
            )
            .context("failed to prepare get_findings_for_task query")?;

        let rows = stmt
            .query_map(params![task_id], |row| {
                Ok(Finding {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    chunk_id: row.get(2)?,
                    finding_type: row.get(3)?,
                    content: row.get(4)?,
                    confidence: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }
}
