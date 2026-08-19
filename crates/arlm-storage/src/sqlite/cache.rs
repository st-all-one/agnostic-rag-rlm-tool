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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_storage() -> (Storage, TempDir) {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        (storage, tmp)
    }

    #[test]
    fn test_get_returns_none_when_empty() {
        let (storage, _tmp) = setup_storage();
        let result = storage.get_cached_result("abc", "proj").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_put_and_get() {
        let (storage, _tmp) = setup_storage();

        storage
            .put_cached_result("hash1", "proj1", "{\"results\":[]}")
            .unwrap();

        let result = storage.get_cached_result("hash1", "proj1").unwrap();
        assert_eq!(result.as_deref(), Some("{\"results\":[]}"));
    }

    #[test]
    fn test_put_overwrites_existing() {
        let (storage, _tmp) = setup_storage();

        storage.put_cached_result("h", "p", "first").unwrap();
        storage.put_cached_result("h", "p", "second").unwrap();

        let result = storage.get_cached_result("h", "p").unwrap();
        assert_eq!(result.as_deref(), Some("second"));
    }

    #[test]
    fn test_different_projects_are_independent() {
        let (storage, _tmp) = setup_storage();

        storage.put_cached_result("h", "p1", "r1").unwrap();
        storage.put_cached_result("h", "p2", "r2").unwrap();

        assert_eq!(
            storage.get_cached_result("h", "p1").unwrap().as_deref(),
            Some("r1")
        );
        assert_eq!(
            storage.get_cached_result("h", "p2").unwrap().as_deref(),
            Some("r2")
        );
    }

    #[test]
    fn test_invalidate_project() {
        let (storage, _tmp) = setup_storage();

        storage.put_cached_result("h1", "proj", "r1").unwrap();
        storage.put_cached_result("h2", "proj", "r2").unwrap();
        storage.put_cached_result("h3", "other", "r3").unwrap();

        let deleted = storage.invalidate_project_cache("proj").unwrap();
        assert_eq!(deleted, 2);

        assert!(storage.get_cached_result("h1", "proj").unwrap().is_none());
        assert!(storage.get_cached_result("h2", "proj").unwrap().is_none());
        assert_eq!(
            storage.get_cached_result("h3", "other").unwrap().as_deref(),
            Some("r3")
        );
    }

    #[test]
    fn test_invalidate_empty_project_returns_zero() {
        let (storage, _tmp) = setup_storage();
        let deleted = storage.invalidate_project_cache("nonexistent").unwrap();
        assert_eq!(deleted, 0);
    }
}
