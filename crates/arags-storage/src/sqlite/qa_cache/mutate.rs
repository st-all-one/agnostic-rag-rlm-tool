//! QA-cache mutation handlers: staleness, deletion, touch, and pending-vector
//! lifecycle for re-embedding.

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter};

use crate::sqlite::conn::Storage;
use tracing::warn;

impl Storage {
    /// Mark an entry stale (soft invalidation) and record the audit trail.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn mark_qa_stale(&self, id: i64, invalidated_by: &str, reason: &str) -> Result<bool> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = crate::sqlite::tokens::now_ms();
        let n: usize = conn
            .execute(|c| {
                let n = c.execute(
                    "UPDATE qa_cache SET stale = 1, confidence = 0, \
                     invalidated_at = ?1, invalidated_by = ?2, invalidated_reason = ?3 \
                     WHERE id = ?4 AND stale = 0",
                    params![now, invalidated_by, reason, id],
                )?;
                Ok(n)
            })
            .context("failed to mark qa_cache stale")?;
        // Staleness triggers a re-digest job so a volunteer re-derives the
        // answer (issue `agnostic-rag-rlm-tool-d172`). Best-effort: a failure to
        // enqueue must not roll back the staleness mark.
        if n > 0 {
            if let Err(e) = self.enqueue_pending_qa_for_stale(id) {
                warn!(error = %e, qa_id = id, "failed to enqueue re-digest job on stale");
            }
        }
        Ok(true)
    }

    /// Hard-delete a single entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete fails.
    pub fn delete_qa(&self, id: i64) -> Result<usize> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let n = c.execute("DELETE FROM qa_cache WHERE id = ?1", params![id])?;
            Ok(n)
        })
        .context("failed to delete qa_cache entry")
    }

    /// Touch an entry on a cache hit: bump `access_count` and `last_accessed_at`.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn touch_qa(&self, id: i64) -> Result<()> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = crate::sqlite::tokens::now_ms();
        conn.execute(|c| {
            c.execute(
                "UPDATE qa_cache SET access_count = access_count + 1, \
                 last_accessed_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            Ok(())
        })
        .context("failed to touch qa_cache entry")
    }

    /// Mark every non-stale entry in a buffer whose `source_hashes` intersect the
    /// given changed hashes as stale. Used by the reindex lifecycle hook.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn mark_stale_by_hashes(&self, buffer_id: i64, changed_hashes: &[String]) -> Result<usize> {
        if changed_hashes.is_empty() {
            return Ok(0);
        }
        let hashes_json =
            serde_json::to_string(changed_hashes).context("failed to serialize changed hashes")?;
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let n = c.execute(
                "UPDATE qa_cache SET stale = 1, confidence = 0 \
                 WHERE buffer_id = ?1 AND stale = 0 \
                 AND EXISTS (SELECT 1 FROM json_each(qa_cache.source_hashes) j \
                     WHERE j.value IN (SELECT value FROM json_each(?2)))",
                params![buffer_id, hashes_json],
            )?;
            Ok(n)
        })
        .context("failed to mark stale by hashes")
    }

    /// Mark the given QA cache rows as awaiting vector re-derivation.
    ///
    /// Sets `vector_status = 'pending_vector'` for every id in `cache_ids`. The
    /// canonical question text is preserved, so a reconcile worker
    /// (issue `agnostic-rag-rlm-tool-36ae`) can re-embed.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn mark_qa_cache_pending_vector(&self, cache_ids: &[i64]) -> Result<()> {
        if cache_ids.is_empty() {
            return Ok(());
        }
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let placeholders: Vec<String> = cache_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect();
            let sql = format!(
                "UPDATE qa_cache SET vector_status = 'pending_vector' WHERE id IN ({})",
                placeholders.join(", ")
            );
            let params: Vec<&dyn rusqlite::ToSql> = cache_ids.iter().map(|id| id as _).collect();
            c.execute(&sql, params_from_iter(params.iter()))
                .context("mark_qa_cache_pending_vector")?;
            Ok(())
        })
    }

    /// Return the IDs of QA cache rows in `project` awaiting vector
    /// re-derivation.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn qa_cache_pending_vector(&self, project: &str) -> Result<Vec<i64>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id FROM qa_cache \
                     WHERE project = ?1 AND vector_status = 'pending_vector'",
                )
                .context("prepare qa_cache_pending_vector")?;
            let rows = stmt
                .query_map(params![project], |row| row.get::<_, i64>(0))
                .context("query qa_cache_pending_vector")?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.context("read qa cache id")?);
            }
            Ok(ids)
        })
    }

    /// Clear the `pending_vector` marker for the given QA cache rows after a
    /// successful re-embed, restoring the normal `indexed` vector status.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn clear_qa_cache_pending_vector(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let placeholders: Vec<String> = ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect();
            let sql = format!(
                "UPDATE qa_cache SET vector_status = 'indexed' WHERE id IN ({})",
                placeholders.join(", ")
            );
            let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|id| id as _).collect();
            c.execute(&sql, params_from_iter(params.iter()))
                .context("clear_qa_cache_pending_vector")?;
            Ok(())
        })
    }
}
