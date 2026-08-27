//! Persist, fetch and search exploration maps (plan 022).

use anyhow::Context as _;
use anyhow::Result;
use rusqlite::OptionalExtension as _;
use rusqlite::params;

use super::super::conn::Storage;
use super::super::tokens::now_ms;
use super::ExplorationRow;
use super::PersistExplorationInput;
use super::StoredExploration;
use super::compress_body;
use super::decompress_body;
use super::parse_stale_reason;

/// Column projection shared by all row queries (order fixed; see
/// [`exploration_mapper`]).
const EXPLORATION_COLS: &str = "id, exploration_id, project, buffer_id, goal, body, summary, \
     created_by, model, template_version, epoch_created, status, stale_reason, confirmed, \
     contradicted, access_count, token_count, created_at, updated_at, last_accessed_at";

fn exploration_mapper(r: &rusqlite::Row<'_>) -> rusqlite::Result<ExplorationRow> {
    Ok(ExplorationRow {
        id: r.get(0)?,
        exploration_id: r.get(1)?,
        project: r.get(2)?,
        buffer_id: r.get(3)?,
        goal: r.get(4)?,
        body: decompress_body(&r.get::<_, Vec<u8>>(5)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Blob, e.into())
        })?,
        summary: r.get(6)?,
        created_by: r.get(7)?,
        model: r.get(8)?,
        template_version: r.get(9)?,
        epoch_created: r.get(10)?,
        status: r.get(11)?,
        stale_reason: parse_stale_reason(r.get::<_, Option<String>>(12)?),
        confirmed: r.get(13)?,
        contradicted: r.get(14)?,
        access_count: r.get(15)?,
        token_count: r.get(16)?,
        created_at: r.get(17)?,
        updated_at: r.get(18)?,
        last_accessed_at: r.get(19)?,
    })
}

impl Storage {
    /// Persist a new exploration map together with its anchors in a single
    /// transaction: either the row and every anchor land, or nothing does.
    ///
    /// The project epoch is stamped on the row so confidence can later measure
    /// drift without extra lookups.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization, compression or any statement fails.
    pub fn persist_exploration(
        &self,
        input: &PersistExplorationInput,
    ) -> Result<StoredExploration> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = now_ms();
        let exploration_id = uuid::Uuid::now_v7().to_string();
        let body = compress_body(&input.body_markdown)?;
        let version = if input.template_version.is_empty() {
            super::TEMPLATE_VERSION_V1
        } else {
            input.template_version.as_str()
        };

        let id: i64 = conn.execute(|c| {
            let tx = c.unchecked_transaction().context("begin exploration tx")?;
            let epoch: i64 = tx
                .query_row(
                    "SELECT COALESCE((SELECT epoch FROM project_epochs WHERE project = ?1), 0)",
                    params![input.project],
                    |r| r.get(0),
                )
                .context("failed to read project epoch")?;

            let rowid: i64 = tx
                .query_row(
                    "INSERT INTO explorations \
                     (exploration_id, project, buffer_id, goal, body, summary, created_by, model, \
                      template_version, epoch_created, status, access_count, token_count, \
                      created_at, updated_at, last_accessed_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'fresh', 0, ?11, ?12, ?12, ?12) \
                     RETURNING id",
                    params![
                        exploration_id,
                        input.project,
                        input.buffer_id,
                        input.goal,
                        body,
                        input.summary,
                        input.created_by,
                        input.model,
                        version,
                        epoch,
                        input.token_count,
                        now,
                    ],
                    |r| r.get(0),
                )
                .context("failed to insert exploration")?;

            for anchor in &input.anchors {
                tx.execute(
                    "INSERT INTO exploration_files \
                     (exploration_rowid, buffer_id, path, content_hash, role) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![rowid, anchor.buffer_id, anchor.path, anchor.content_hash, anchor.role],
                )
                .with_context(|| format!("failed to anchor {}", anchor.path))?;
            }

            tx.commit().context("commit exploration tx")?;
            Ok(rowid)
        })?;
        tracing::debug!(
            rowid = id,
            exploration_id = %exploration_id,
            project = %input.project,
            anchors = input.anchors.len(),
            bytes = body.len(),
            "exploration persisted"
        );

        Ok(StoredExploration { exploration_id, id })
    }

    /// Fetch a map by its stable UUIDv7 id.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_exploration_by_uuid(&self, exploration_id: &str) -> Result<Option<ExplorationRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let sql =
                format!("SELECT {EXPLORATION_COLS} FROM explorations WHERE exploration_id = ?1");
            c.query_row(&sql, params![exploration_id], exploration_mapper)
                .optional()
                .context("failed to get exploration by uuid")
        })
    }

    /// Fetch a map by numeric rowid (vector-space key).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_exploration_by_rowid(&self, id: i64) -> Result<Option<ExplorationRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let sql = format!("SELECT {EXPLORATION_COLS} FROM explorations WHERE id = ?1");
            c.query_row(&sql, params![id], exploration_mapper)
                .optional()
                .context("failed to get exploration by rowid")
        })
    }

    /// Lexical FTS search scoped to a project. The caller must pass an
    /// already-sanitized FTS5 query (see `grpc::util::sanitize_fts`). Only
    /// non-retired maps are returned regardless of staleness; ordering follows
    /// BM25 rank.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn search_explorations_fts(
        &self,
        project: &str,
        fts_query: &str,
        limit: usize,
    ) -> Result<Vec<ExplorationRow>> {
        if fts_query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            #[allow(clippy::cast_possible_wrap)] // limit is small
            let limit_i64 = limit as i64;
            let sql = format!(
                "SELECT {EXPLORATION_COLS} FROM explorations \
                 WHERE explorations.rowid IN \
                   (SELECT rowid FROM explorations_fts WHERE explorations_fts MATCH ?2 \
                    ORDER BY rank LIMIT ?3) \
                 AND project = ?1 AND status != 'retired'"
            );
            let mut stmt = c.prepare(&sql).context("prepare search_explorations_fts")?;
            let rows = stmt
                .query_map(params![project, fts_query, limit_i64], exploration_mapper)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Bump `access_count` and `last_accessed_at` on a served hit.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn touch_exploration(&self, id: i64) -> Result<()> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = now_ms();
        conn.execute(|c| {
            c.execute(
                "UPDATE explorations SET access_count = access_count + 1, last_accessed_at = ?1 \
                 WHERE id = ?2",
                params![now, id],
            )?;
            Ok(())
        })
        .context("failed to touch exploration")
    }

    /// Count maps for a project, optionally restricted to one lifecycle state.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn count_explorations(&self, project: &str, status: Option<&str>) -> Result<usize> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let n: i64 = match status {
                Some(s) => c.query_row(
                    "SELECT COUNT(*) FROM explorations WHERE project = ?1 AND status = ?2",
                    params![project, s],
                    |r| r.get(0),
                )?,
                None => c.query_row(
                    "SELECT COUNT(*) FROM explorations WHERE project = ?1",
                    params![project],
                    |r| r.get(0),
                )?,
            };
            Ok(usize::try_from(n).unwrap_or(0))
        })
    }

    /// Hard-delete a map (admin `Delete` mode). Anchors and the FTS row are
    /// removed by cascade/triggers; the caller owns vector removal.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete fails.
    pub fn delete_exploration(&self, id: i64) -> Result<usize> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let n = c.execute("DELETE FROM explorations WHERE id = ?1", params![id])?;
            Ok(n)
        })
        .context("failed to delete exploration")
    }

    /// Mark the given explorations as awaiting vector re-derivation.
    ///
    /// Sets `vector_status = 'pending_vector'` for every id in `exploration_ids`
    /// that belongs to `buffer_id`. The canonical summary text is preserved, so
    /// a reconcile worker (issue `agnostic-rlm-rs-36ae`) can re-embed.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn mark_explorations_pending_vector(
        &self,
        buffer_id: i64,
        exploration_ids: &[i64],
    ) -> Result<()> {
        if exploration_ids.is_empty() {
            return Ok(());
        }
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let placeholders: Vec<String> = exploration_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect();
            let sql = format!(
                "UPDATE explorations SET vector_status = 'pending_vector' \
                 WHERE buffer_id = ?1 AND id IN ({})",
                placeholders.join(", ")
            );
            let mut params: Vec<&dyn rusqlite::ToSql> = vec![&buffer_id];
            for id in exploration_ids {
                params.push(id);
            }
            c.execute(&sql, rusqlite::params_from_iter(params.iter()))
                .context("mark_explorations_pending_vector")?;
            Ok(())
        })
    }

    /// Return the IDs of explorations in `buffer_id` awaiting vector
    /// re-derivation.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn explorations_pending_vector(&self, buffer_id: i64) -> Result<Vec<i64>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id FROM explorations \
                     WHERE buffer_id = ?1 AND vector_status = 'pending_vector'",
                )
                .context("prepare explorations_pending_vector")?;
            let rows = stmt
                .query_map(params![buffer_id], |row| row.get::<_, i64>(0))
                .context("query explorations_pending_vector")?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.context("read exploration id")?);
            }
            Ok(ids)
        })
    }
}
