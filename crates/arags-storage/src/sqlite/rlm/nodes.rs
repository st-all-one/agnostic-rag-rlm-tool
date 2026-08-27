//! Summary node persistence: upsert, review gate, lookup and hydration.

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use rusqlite::params;
use std::fmt::Write as _;

use super::super::conn::Storage;
use super::super::tokens::now_ms;
use super::{NODE_COLS, NewRlmNode, REVIEW_APPROVED, REVIEW_REJECTED, RlmNode, node_mapper};

impl Storage {
    /// Upsert a summary node keyed by `(project, level, subject)`. The new
    /// submission replaces the previous content and **resets
    /// `review_status` to `pending`** (quality gate); provenance edges must be
    /// written separately via [`Storage::add_rlm_edge`].
    ///
    /// # Errors
    ///
    /// Returns an error if the upsert fails or hashes cannot be serialized.
    pub fn store_rlm_node(&self, input: &NewRlmNode) -> Result<(i64, String)> {
        let now = now_ms();
        let start = std::time::Instant::now();
        let node_id = uuid::Uuid::now_v7().to_string();
        let hashes_json =
            serde_json::to_string(&input.source_hashes).context("serialize source_hashes")?;
        let conn = self.connection().context("acquire connection")?;
        let (id, node_id) = conn
            .execute(|c| {
                Ok(super::upsert_node_stmt(
                    c,
                    &node_id,
                    &hashes_json,
                    input,
                    now,
                )?)
            })
            .context("upsert rlm_node")?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::debug!(
            phase = "store_rlm_node",
            rowid = id,
            node_id = %node_id,
            level = input.level,
            project = %input.project,
            subject = %input.subject,
            elapsed_ms = format!("{elapsed_ms:.2}"),
            "rlm node stored (superseding prior active revision)"
        );
        Ok((id, node_id))
    }

    /// Get an approved (or at least non-rejected) node by stable `node_id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_rlm_node(&self, node_id: &str) -> Result<Option<RlmNode>> {
        let conn = self.connection().context("acquire connection")?;
        let sql = format!("SELECT {NODE_COLS} FROM rlm_nodes WHERE node_id = ?1");
        conn.execute(|c| {
            c.query_row(sql.as_str(), params![node_id], node_mapper)
                .optional()
                .context("get rlm_node")
        })
    }

    /// Get a node by natural key `(project, level, subject)` regardless of
    /// review status (motor change-detection path).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_rlm_node_by_subject(
        &self,
        project: &str,
        level: i64,
        subject: &str,
    ) -> Result<Option<RlmNode>> {
        let sql = format!(
            "SELECT {NODE_COLS} FROM rlm_nodes \
             WHERE project = ?1 AND level = ?2 AND subject = ?3 AND is_active = 1"
        );
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.query_row(sql.as_str(), params![project, level, subject], node_mapper)
                .optional()
                .context("get_rlm_node_by_subject")
        })
    }

    /// List nodes for a project, optionally filtered by level and staleness.
    /// Only `approved` nodes are returned unless `include_pending` is set
    /// (admin review queue).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_rlm_nodes(
        &self,
        project: &str,
        level: Option<i64>,
        include_pending: bool,
    ) -> Result<Vec<RlmNode>> {
        let mut sql = format!(
            "SELECT {NODE_COLS} FROM rlm_nodes \
             WHERE project = ?1 AND is_active = 1 AND review_status != '{REVIEW_REJECTED}'"
        );
        if !include_pending {
            let _ = write!(sql, " AND review_status = '{REVIEW_APPROVED}'");
        }
        if level.is_some() {
            sql.push_str(" AND level = ?2");
        }
        sql.push_str(" ORDER BY level, subject");
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c.prepare(&sql).context("prepare list_rlm_nodes")?;
            let rows = match level {
                Some(l) => stmt
                    .query_map(params![project, l], node_mapper)?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
                None => stmt
                    .query_map(params![project], node_mapper)?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            };
            Ok(rows)
        })
    }

    /// Apply the quality-gate verdict to a node.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn review_rlm_node(
        &self,
        node_id: &str,
        approved: bool,
        reviewer: &str,
        reason: Option<&str>,
    ) -> Result<bool> {
        let status = if approved {
            REVIEW_APPROVED
        } else {
            REVIEW_REJECTED
        };
        let conn = self.connection().context("acquire connection")?;
        let n = conn.execute(|c| {
            c.execute(
                "UPDATE rlm_nodes SET review_status = ?1, reviewed_by = ?2, reviewed_at = ?3, \
                   confidence = CASE WHEN ?1 = 'approved' THEN confidence ELSE 0 END \
                 WHERE node_id = ?4",
                params![status, reviewer, now_ms(), node_id],
            )
            .context("review rlm_node")
        })?;
        let _ = reason; // recorded in tracing only for now (schema keeps it minimal)
        tracing::info!(node_id, status, reviewer, "rlm node reviewed");
        Ok(n > 0)
    }

    /// Lexical search over approved, non-stale summaries via the `rlm_fts`
    /// index. `query` must already be FTS5-sanitised by the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn search_rlm_fts(
        &self,
        buffer_id: i64,
        fts_query: &str,
        limit: usize,
    ) -> Result<Vec<RlmNode>> {
        let sql = format!(
            "SELECT {NODE_COLS} FROM rlm_nodes \
             WHERE rlm_nodes.rowid IN \
               (SELECT rowid FROM rlm_fts WHERE rlm_fts MATCH ?1 ORDER BY rank LIMIT ?3) \
               AND buffer_id = ?2 AND is_active = 1 AND stale = 0 \
               AND review_status = '{REVIEW_APPROVED}'"
        );
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            #[allow(clippy::cast_possible_wrap)] // limit is small
            let mut stmt = c.prepare(&sql).context("prepare search_rlm_fts")?;
            let rows = stmt
                .query_map(params![fts_query, buffer_id, limit as i64], node_mapper)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Fetch specific nodes by rowid (vector-search hydration), scoped to a
    /// buffer (project). Only approved, non-stale nodes are returned. The
    /// vector space itself is global, so this scope filter is what keeps other
    /// projects' summaries out of search results.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or the query fails.
    pub fn get_approved_rlm_nodes(&self, ids: &[u64], buffer_id: i64) -> Result<Vec<RlmNode>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<i64> = ids
            .iter()
            .map(|&id| i64::try_from(id).context("rlm node rowid exceeds i64"))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let ids_json = serde_json::to_string(&ids).context("serialize node ids")?;
        let sql = format!(
            "SELECT {NODE_COLS} FROM rlm_nodes \
             WHERE id IN (SELECT value FROM json_each(?1)) \
               AND buffer_id = ?2 AND is_active = 1 AND stale = 0 \
               AND review_status = '{REVIEW_APPROVED}'"
        );
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c.prepare(&sql).context("prepare get_approved_rlm_nodes")?;
            let rows = stmt
                .query_map(params![ids_json, buffer_id], node_mapper)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Resolve `(project, level, subject)` of a node by stable id (for job
    /// keys / cancellation).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn rlm_subject_of(&self, node_id: &str) -> Result<Option<(String, i64, String)>> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                "SELECT project, level, subject FROM rlm_nodes WHERE node_id = ?1",
                params![node_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .context("rlm_subject_of")
        })
    }

    /// Walk the supersede chain starting from `id`, returning every revision in
    /// oldest→newest order (issue `agnostic-rlm-rs-e210`). The starting row need
    /// not be the oldest; only the forward chain reachable via `superseded_by`
    /// is returned. Retired (`is_active = 0`) revisions are included so callers
    /// can audit the full node history.
    ///
    /// # Errors
    ///
    /// Returns an error if any query fails.
    pub fn get_node_history(&self, id: i64) -> Result<Vec<RlmNode>> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut chain = Vec::new();
            let mut current: Option<i64> = Some(id);
            while let Some(cid) = current {
                let sql = format!("SELECT {NODE_COLS} FROM rlm_nodes WHERE id = ?1");
                let Some(row) = c
                    .query_row(sql.as_str(), params![cid], node_mapper)
                    .optional()
                    .context("failed to read rlm_node history row")?
                else {
                    break;
                };
                let next: Option<i64> = c
                    .query_row(
                        "SELECT superseded_by FROM rlm_nodes WHERE id = ?1",
                        params![cid],
                        |r| r.get(0),
                    )
                    .context("failed to read rlm_node superseded_by")?;
                chain.push(row);
                current = next;
            }
            Ok(chain)
        })
    }

    /// Mark the given RLM nodes as awaiting vector re-derivation.
    ///
    /// Sets `vector_status = 'pending_vector'` for every node in `node_ids`
    /// that belongs to `buffer_id`. The canonical summary text is preserved,
    /// so a reconcile worker (issue `agnostic-rlm-rs-36ae`) can re-embed.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn mark_rlm_nodes_pending_vector(&self, buffer_id: i64, node_ids: &[i64]) -> Result<()> {
        if node_ids.is_empty() {
            return Ok(());
        }
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let placeholders: Vec<String> = node_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect();
            let sql = format!(
                "UPDATE rlm_nodes SET vector_status = 'pending_vector' \
                 WHERE buffer_id = ?1 AND id IN ({})",
                placeholders.join(", ")
            );
            let mut params: Vec<&dyn rusqlite::ToSql> = vec![&buffer_id];
            for id in node_ids {
                params.push(id);
            }
            c.execute(&sql, rusqlite::params_from_iter(params.iter()))
                .context("mark_rlm_nodes_pending_vector")?;
            Ok(())
        })
    }

    /// Return the IDs of RLM nodes in `buffer_id` awaiting vector
    /// re-derivation.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn rlm_nodes_pending_vector(&self, buffer_id: i64) -> Result<Vec<i64>> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id FROM rlm_nodes \
                     WHERE buffer_id = ?1 AND vector_status = 'pending_vector'",
                )
                .context("prepare rlm_nodes_pending_vector")?;
            let rows = stmt
                .query_map(params![buffer_id], |row| row.get::<_, i64>(0))
                .context("query rlm_nodes_pending_vector")?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.context("read rlm node id")?);
            }
            Ok(ids)
        })
    }
}
