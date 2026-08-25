//! Provenance edges and staleness invalidation for RLM nodes.
//!
//! `rlm_edges` records which chunks/child nodes feed a parent summary;
//! invalidation walks those edges upward and marks affected nodes stale when
//! their recorded `source_hashes` intersect the changed hashes.

use anyhow::{Context, Result};
use rusqlite::params;

use super::super::conn::Storage;

impl Storage {
    /// Record a provenance edge. Exactly one of `child_node_id`/`chunk_id`
    /// must be `Some`.
    ///
    /// # Errors
    ///
    /// Returns an error if both/neither reference is set or the insert fails.
    pub fn add_rlm_edge(
        &self,
        parent_rowid: i64,
        child_node_id: Option<i64>,
        chunk_id: Option<i64>,
    ) -> Result<()> {
        anyhow::ensure!(
            child_node_id.is_some() != chunk_id.is_some(),
            "rlm edge needs exactly one of child_node_id/chunk_id"
        );
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.execute(
                "INSERT OR IGNORE INTO rlm_edges (parent_id, child_node_id, chunk_id) \
                 VALUES (?1, ?2, ?3)",
                params![parent_rowid, child_node_id, chunk_id],
            )
            .context("insert rlm_edge")
        })?;
        Ok(())
    }

    /// Resolve the parent chain bottom-up: which node rowids depend directly
    /// or transitively on the given node rowids.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or the query fails.
    pub fn rlm_parent_chain(&self, node_ids: &[i64]) -> Result<Vec<i64>> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids_json = serde_json::to_string(node_ids).context("serialize node ids")?;
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            // Recursive CTE walking child -> parent edges upward; the seed and
            // the exclusion set are the same bound JSON array (no interpolation).
            let sql = "WITH RECURSIVE up(id) AS ( \
                         SELECT value FROM json_each(?1) \
                         UNION \
                         SELECT e.parent_id FROM rlm_edges e \
                           JOIN up ON e.child_node_id = up.id \
                       ) SELECT DISTINCT id FROM up \
                         WHERE id NOT IN (SELECT value FROM json_each(?1))";
            let mut stmt = c.prepare(sql).context("prepare rlm_parent_chain")?;
            let rows = stmt
                .query_map(params![ids_json], |r| r.get::<_, i64>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Mark nodes stale when their recorded `source_hashes` intersect the
    /// changed hashes (same mechanism as qa_cache). Returns affected rows as
    /// `(rowid, project, level, subject)` so the caller can enqueue rework.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or the update fails.
    pub fn mark_rlm_stale_by_hashes(
        &self,
        buffer_id: i64,
        changed_hashes: &[String],
    ) -> Result<Vec<(i64, String, i64, String)>> {
        if changed_hashes.is_empty() {
            return Ok(Vec::new());
        }
        let hashes_json =
            serde_json::to_string(changed_hashes).context("serialize changed hashes")?;
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id, project, level, subject FROM rlm_nodes \
                     WHERE buffer_id = ?1 AND stale = 0 \
                     AND EXISTS (SELECT 1 FROM json_each(rlm_nodes.source_hashes) j \
                         WHERE j.value IN (SELECT value FROM json_each(?2)))",
                )
                .context("prepare select stale rlm")?;
            let affected = stmt
                .query_map(params![buffer_id, hashes_json], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .context("query stale rlm")?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if !affected.is_empty() {
                c.execute(
                    "UPDATE rlm_nodes SET stale = 1, confidence = 0 \
                     WHERE buffer_id = ?1 AND stale = 0 \
                     AND EXISTS (SELECT 1 FROM json_each(rlm_nodes.source_hashes) j \
                         WHERE j.value IN (SELECT value FROM json_each(?2)))",
                    params![buffer_id, hashes_json],
                )
                .context("mark rlm stale")?;
            }
            Ok(affected)
        })
    }

    /// Mark a single node stale by `(buffer_id, level, subject)` — used when
    /// the motor already knows which subjects changed. Returns whether a live
    /// node was affected.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn mark_rlm_stale_by_subject(
        &self,
        buffer_id: i64,
        level: i64,
        subject: &str,
    ) -> Result<bool> {
        let conn = self.connection().context("acquire connection")?;
        let n = conn.execute(|c| {
            c.execute(
                "UPDATE rlm_nodes SET stale = 1, confidence = 0 \
                 WHERE buffer_id = ?1 AND level = ?2 AND subject = ?3 AND stale = 0",
                params![buffer_id, level, subject],
            )
            .context("mark_rlm_stale_by_subject")
        })?;
        Ok(n > 0)
    }

    /// Current chunk snapshot of a file: `(chunk_id, sha256 hex hash, text)`.
    /// Drives the L1 job payload and change detection; lives here because it
    /// is the input to staleness/enqueue decisions.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn rlm_chunks_snapshot(
        &self,
        buffer_id: i64,
        file_path: &str,
    ) -> Result<Vec<(i64, String, Option<String>)>> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT c.id, hex(c.hash), t.content FROM chunks c \
                     LEFT JOIN chunk_texts t ON t.chunk_id = c.id \
                     WHERE c.buffer_id = ?1 AND c.file_path = ?2 ORDER BY c.id",
                )
                .context("prepare rlm_chunks_snapshot")?;
            let rows = stmt
                .query_map(params![buffer_id, file_path], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}
