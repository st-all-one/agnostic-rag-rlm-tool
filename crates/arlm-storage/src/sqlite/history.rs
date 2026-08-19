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
    fn test_insert_history() {
        let (storage, _tmp) = setup_storage();

        let id = storage
            .insert_history(
                None,
                "bug in login",
                Some("search"),
                Some(5),
                Some(100),
                Some("opencode"),
            )
            .unwrap();
        assert!(id > 0);

        let entries = storage.get_history(None, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].query, "bug in login");
    }
}
