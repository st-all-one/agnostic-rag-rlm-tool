//! Hierarchical summary persistence and project summary statistics.

use anyhow::{Context, Result};
use arlm_storage::Storage;
use rusqlite::params;

/// Summary counts for a project, grouped by scope.
#[derive(Debug, Clone, Default)]
pub struct SummaryCounts {
    pub total: i64,
    pub file: i64,
    pub module: i64,
    pub project: i64,
}

/// Count summaries for a project by scope.
///
/// # Errors
///
/// Returns an error if any query fails.
pub fn summary_counts(storage: &Storage, project: &str) -> Result<SummaryCounts> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        const SCOPE_BASE: &str =
            "SELECT COUNT(*) FROM summaries WHERE buffer_id IN (SELECT id FROM buffers WHERE name = ?1)";
        let count_scope = |conn: &rusqlite::Connection, scope: &str| -> rusqlite::Result<i64> {
            if scope.is_empty() {
                conn.query_row(SCOPE_BASE, params![project], |row| row.get(0))
            } else {
                conn.query_row(
                    &format!("{SCOPE_BASE} AND scope = ?2"),
                    params![project, scope],
                    |row| row.get(0),
                )
            }
        };

        let total = count_scope(conn, "").unwrap_or(0);
        let file = count_scope(conn, "file").unwrap_or(0);
        let module = count_scope(conn, "module").unwrap_or(0);
        let project_count = count_scope(conn, "project").unwrap_or(0);
        Ok(SummaryCounts {
            total,
            file,
            module,
            project: project_count,
        })
    })
    .context("failed to count summaries")
}

/// Total number of summaries across all projects.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn count_all_summaries(storage: &Storage) -> Result<i64> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM summaries", [], |row| row.get(0))?))
        .context("failed to count all summaries")
}

/// Insert a hierarchical summary record.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_summary(
    storage: &Storage,
    buffer_id: i64,
    content: &str,
    scope: &str,
    source_chunk_ids: &str,
    source_hash: &str,
    confidence: f64,
    tokens: u32,
) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO summaries (buffer_id, content, scope, source_chunk_ids, source_hash, confidence, tokens) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                buffer_id,
                content,
                scope,
                source_chunk_ids,
                source_hash,
                confidence,
                tokens,
            ],
        )?;
        Ok(())
    })
    .context("failed to insert summary")
}
