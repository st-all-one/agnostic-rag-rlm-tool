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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_storage() -> (Storage, TempDir) {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        (storage, tmp)
    }

    #[test]
    fn test_insert_finding() {
        let (storage, _tmp) = setup_storage();

        let conn = storage.conn();
        let conn = conn.lock();
        conn.execute(
            "INSERT INTO buffers (name, path) VALUES ('test', '/test')",
            [],
        )
        .unwrap();
        let buffer_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO tasks (buffer_id) VALUES (?1)",
            params![buffer_id],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();
        drop(conn);

        let finding_id = storage
            .insert_finding(task_id, None, Some("bug"), "Found a bug", Some(0.9))
            .unwrap();
        assert!(finding_id > 0);

        let findings = storage.get_findings_for_task(task_id).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].finding_type, Some("bug".to_string()));
    }
}
