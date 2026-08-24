use anyhow::{Context, Result};
use rusqlite::params;

use super::conn::Storage;

/// Query history entry.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    pub buffer_id: Option<i64>,
    pub query: String,
    pub query_type: Option<String>,
    pub results_count: Option<i64>,
    pub duration_ms: Option<i64>,
    pub used_by: Option<String>,
    pub result_hash: Option<Vec<u8>>,
    pub created_at: i64,
}

impl Storage {
    /// Insert a history entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub fn insert_history(
        &self,
        buffer_id: Option<i64>,
        query: &str,
        query_type: Option<&str>,
        results_count: Option<i64>,
        duration_ms: Option<i64>,
        used_by: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn();
        let conn = conn.lock();

        let id = conn
            .execute(
                "INSERT INTO history (buffer_id, query, query_type, results_count, duration_ms, used_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![buffer_id, query, query_type, results_count, duration_ms, used_by],
            )
            .context("failed to insert history")?;

        let history_id = i64::try_from(id).context("history id overflow")?;
        tracing::info!(history_id, query_type, used_by, "inserted history entry");

        Ok(history_id)
    }

    /// Get recent history entries.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_history(&self, buffer_id: Option<i64>, limit: i64) -> Result<Vec<HistoryEntry>> {
        let conn = self.conn();
        let conn = conn.lock();

        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(bid) =
            buffer_id
        {
            (
                "SELECT id, buffer_id, query, query_type, results_count, duration_ms, used_by, result_hash, created_at FROM history WHERE buffer_id = ?1 ORDER BY created_at DESC LIMIT ?2".to_string(),
                vec![Box::new(bid), Box::new(limit)],
            )
        } else {
            (
                "SELECT id, buffer_id, query, query_type, results_count, duration_ms, used_by, result_hash, created_at FROM history ORDER BY created_at DESC LIMIT ?1".to_string(),
                vec![Box::new(limit)],
            )
        };

        let mut stmt = conn
            .prepare(&sql)
            .context("failed to prepare get_history query")?;

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(AsRef::as_ref).collect();

        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    buffer_id: row.get(1)?,
                    query: row.get(2)?,
                    query_type: row.get(3)?,
                    results_count: row.get(4)?,
                    duration_ms: row.get(5)?,
                    used_by: row.get(6)?,
                    result_hash: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }
}

impl Storage {
    /// Delete history entries older than `cutoff_unix` (epoch seconds),
    /// returning how many rows were removed. Used by the server's
    /// `[history] retention_days` maintenance (plan 020).
    ///
    /// # Errors
    ///
    /// Returns an error if the delete fails.
    pub fn purge_history_before(&self, cutoff_unix: i64) -> Result<u64> {
        let conn = self.conn();
        let conn = conn.lock();

        let n = conn
            .execute(
                "DELETE FROM history WHERE created_at < ?1",
                params![cutoff_unix],
            )
            .context("failed to purge history")?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_purge_history_before_removes_only_old_rows() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let old = now - 10 * 86_400;

        // Seed one old and one current row by inserting then backdating.
        storage
            .insert_history(None, "old", Some("search"), None, None, None)
            .unwrap();
        storage
            .insert_history(None, "new", Some("search"), None, None, None)
            .unwrap();

        let conn = storage.conn();
        let guard = conn.lock();
        guard
            .execute(
                "UPDATE history SET created_at = ?1 WHERE query = 'old'",
                params![old],
            )
            .unwrap();
        drop(guard);

        let removed = storage.purge_history_before(now - 86_400).unwrap();
        assert_eq!(removed, 1);

        let remaining: Vec<HistoryEntry> = storage.get_history(None, 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].query, "new");
    }
}
