use anyhow::{Context, Result};
use rusqlite::params;

use arags_storage::Storage;

use crate::ScopedTimer;

/// Internal scope selector for history queries.
enum UserScope<'a> {
    Buffer(i64),
    Name(&'a str),
}

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
    pub user: Option<String>,
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
        self.record_with_user(
            buffer_id,
            query,
            query_type,
            results_count,
            duration_ms,
            used_by,
            "",
        )
    }

    /// Record a query against a project, attributing it to `user` (plan 019, E).
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    #[allow(clippy::too_many_arguments)]
    pub fn record_with_user(
        &self,
        buffer_id: Option<i64>,
        query: &str,
        query_type: Option<&str>,
        results_count: Option<i64>,
        duration_ms: Option<i64>,
        used_by: Option<&str>,
        user: &str,
    ) -> Result<i64> {
        let _timer = ScopedTimer::new("history_record");

        let conn = self.storage.conn();
        let conn = conn.lock();

        let id = conn
            .execute(
                "INSERT INTO history (buffer_id, query, query_type, results_count, duration_ms, used_by, user) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    buffer_id,
                    query,
                    query_type,
                    results_count,
                    duration_ms,
                    used_by,
                    if user.is_empty() {
                        None::<String>
                    } else {
                        Some(user.to_string())
                    }
                ],
            )
            .context("failed to insert history")?;

        let history_id = i64::try_from(id).context("history id overflow")?;
        tracing::info!(history_id, query_type, used_by, user, "query recorded");

        Ok(history_id)
    }

    /// Get recent query history for a project.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn recent(&self, buffer_id: Option<i64>, limit: i64) -> Result<Vec<QueryRecord>> {
        self.recent_opt_user_internal(buffer_id.map(UserScope::Buffer), limit)
    }

    /// Get recent query history, optionally scoped to a single `user` (plan 019,
    /// E). An empty `user` returns history for all users (admin scope).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn recent_opt_user(&self, user: Option<&str>, limit: i64) -> Result<Vec<QueryRecord>> {
        self.recent_opt_user_internal(user.filter(|u| !u.is_empty()).map(UserScope::Name), limit)
    }

    #[allow(clippy::needless_pass_by_value)]
    fn recent_opt_user_internal(
        &self,
        scope: Option<UserScope<'_>>,
        limit: i64,
    ) -> Result<Vec<QueryRecord>> {
        let _timer = ScopedTimer::new("history_recent");

        let conn = self.storage.conn();
        let conn = conn.lock();

        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match scope {
            Some(UserScope::Buffer(bid)) => (
                "SELECT id, buffer_id, query, query_type, results_count, duration_ms, used_by, user, created_at \
                 FROM history WHERE buffer_id = ?1 ORDER BY created_at DESC LIMIT ?2"
                    .to_string(),
                vec![Box::new(bid), Box::new(limit)],
            ),
            Some(UserScope::Name(user)) => (
                "SELECT id, buffer_id, query, query_type, results_count, duration_ms, used_by, user, created_at \
                 FROM history WHERE user = ?1 ORDER BY created_at DESC LIMIT ?2"
                    .to_string(),
                vec![Box::new(user), Box::new(limit)],
            ),
            None => (
                "SELECT id, buffer_id, query, query_type, results_count, duration_ms, used_by, user, created_at \
                 FROM history ORDER BY created_at DESC LIMIT ?1"
                    .to_string(),
                vec![Box::new(limit)],
            ),
        };

        let mut stmt = conn
            .prepare(&sql)
            .context("failed to prepare history query")?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(AsRef::as_ref).collect();

        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(QueryRecord {
                    id: row.get(0)?,
                    buffer_id: row.get(1)?,
                    query: row.get(2)?,
                    query_type: row.get(3)?,
                    results_count: row.get(4)?,
                    duration_ms: row.get(5)?,
                    used_by: row.get(6)?,
                    user: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
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
