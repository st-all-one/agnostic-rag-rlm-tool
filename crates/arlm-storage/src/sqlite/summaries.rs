use anyhow::{Context, Result};
use rusqlite::params;

use super::conn::Storage;

/// Hierarchical summary of code (file / module / project scope).
#[derive(Debug, Clone)]
pub struct Summary {
    pub id: i64,
    pub buffer_id: i64,
    pub content: String,
    pub scope: String,
    pub source_chunk_ids: Option<Vec<i64>>,
    pub source_hash: Option<String>,
    pub confidence: f64,
    pub version: i64,
    pub tokens: Option<i64>,
    pub parent_summary_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

const SUMMARY_COLUMNS: &str = "id, buffer_id, content, scope, source_chunk_ids, source_hash, confidence, version, tokens, parent_summary_id, created_at, updated_at";

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<Summary> {
    let raw_ids: Option<String> = row.get(4)?;
    let source_chunk_ids = raw_ids.and_then(|s| serde_json::from_str::<Vec<i64>>(&s).ok());
    Ok(Summary {
        id: row.get(0)?,
        buffer_id: row.get(1)?,
        content: row.get(2)?,
        scope: row.get(3)?,
        source_chunk_ids,
        source_hash: row.get(5)?,
        confidence: row.get(6)?,
        version: row.get(7)?,
        tokens: row.get(8)?,
        parent_summary_id: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

impl Storage {
    /// Insert a summary for a buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if serializing the source chunk ids or the insert fails.
    pub fn insert_summary(
        &self,
        buffer_id: i64,
        content: &str,
        scope: &str,
        source_chunk_ids: Option<&[i64]>,
        source_hash: Option<&str>,
        confidence: f64,
        tokens: Option<i64>,
        parent_summary_id: Option<i64>,
    ) -> Result<i64> {
        let source_chunk_ids_json = source_chunk_ids
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialize source chunk ids")?;

        let conn = self.conn();
        let conn = conn.lock();
        conn.execute(
            "INSERT INTO summaries (buffer_id, content, scope, source_chunk_ids, source_hash, confidence, tokens, parent_summary_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                buffer_id,
                content,
                scope,
                source_chunk_ids_json,
                source_hash,
                confidence,
                tokens,
                parent_summary_id
            ],
        )
        .context("failed to insert summary")?;

        let id = conn.last_insert_rowid();
        tracing::info!(summary_id = id, buffer_id, scope, "inserted summary");
        Ok(id)
    }

    /// Get all summaries for a buffer, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_summaries(&self, buffer_id: i64) -> Result<Vec<Summary>> {
        let conn = self.conn();
        let conn = conn.lock();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SUMMARY_COLUMNS} FROM summaries WHERE buffer_id = ?1 ORDER BY id DESC"
            ))
            .context("failed to prepare get_summaries")?;

        let rows = stmt
            .query_map(params![buffer_id], row_to_summary)?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(rows)
    }

    /// Get the project-level summary for a buffer, if any.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_project_summary(&self, buffer_id: i64) -> Result<Option<Summary>> {
        let conn = self.conn();
        let conn = conn.lock();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SUMMARY_COLUMNS} FROM summaries WHERE buffer_id = ?1 AND scope = 'project' ORDER BY id DESC LIMIT 1"
            ))
            .context("failed to prepare get_project_summary")?;

        let mut rows = stmt.query_map(params![buffer_id], row_to_summary)?;
        rows.next().transpose().context("failed to read project summary")
    }

    /// Get a summary by its source hash (used for incremental refresh).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_summary_by_source_hash(
        &self,
        buffer_id: i64,
        source_hash: &str,
    ) -> Result<Option<Summary>> {
        let conn = self.conn();
        let conn = conn.lock();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SUMMARY_COLUMNS} FROM summaries WHERE buffer_id = ?1 AND source_hash = ?2 ORDER BY id DESC LIMIT 1"
            ))
            .context("failed to prepare get_summary_by_source_hash")?;

        let mut rows = stmt
            .query_map(params![buffer_id, source_hash], row_to_summary)?;
        rows.next()
            .transpose()
            .context("failed to read summary by source hash")
    }
}
