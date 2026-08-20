use anyhow::{Context, Result};

use arlm_storage::Storage;

use crate::ScopedTimer;

/// A recorded query with metadata.
#[derive(Debug, Clone)]
pub struct QueryRecord {
    pub id: i64,
    pub buffer_id: Option<i64>,
    pub query: String,
    pub query_type: Option<String>,
    pub results_count: Option<i64>,
    pub duration_ms: Option<i64>,
    pub used_by: Option<String>,
    pub created_at: i64,
}

/// Manages query history for projects.
pub struct HistoryManager {
    storage: Storage,
}

impl HistoryManager {
    /// Create a new `HistoryManager`.
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    /// Record a query against a project.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn record(
        &self,
        buffer_id: Option<i64>,
        query: &str,
        query_type: Option<&str>,
        results_count: Option<i64>,
        duration_ms: Option<i64>,
        used_by: Option<&str>,
    ) -> Result<i64> {
        let _timer = ScopedTimer::new("history_record");

        let id = self
            .storage
            .insert_history(
                buffer_id,
                query,
                query_type,
                results_count,
                duration_ms,
                used_by,
            )
            .context("failed to record query")?;

        tracing::info!(history_id = id, query_type, used_by, "query recorded");

        Ok(id)
    }

    /// Get recent query history for a project.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn recent(&self, buffer_id: Option<i64>, limit: i64) -> Result<Vec<QueryRecord>> {
        let _timer = ScopedTimer::new("history_recent");

        let entries = self
            .storage
            .get_history(buffer_id, limit)
            .context("failed to get history")?;

        Ok(entries
            .into_iter()
            .map(|e| QueryRecord {
                id: e.id,
                buffer_id: e.buffer_id,
                query: e.query,
                query_type: e.query_type,
                results_count: e.results_count,
                duration_ms: e.duration_ms,
                used_by: e.used_by,
                created_at: e.created_at,
            })
            .collect())
    }

    /// Count total queries for a project.
    ///
    /// # Errors
    ///
    /// Returns an error if the count fails.
    pub fn count(&self, buffer_id: Option<i64>) -> Result<i64> {
        let conn = self.storage.conn();
        let conn = conn.lock();

        let count: i64 = if let Some(bid) = buffer_id {
            conn.query_row(
                "SELECT COUNT(*) FROM history WHERE buffer_id = ?1",
                [bid],
                |row| row.get(0),
            )
            .context("failed to count history")?
        } else {
            conn.query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
                .context("failed to count history")?
        };

        Ok(count)
    }
}
