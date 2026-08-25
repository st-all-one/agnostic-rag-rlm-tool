//! Project epochs and anchor-based staleness (plan 022).
//!
//! Two complementary signals:
//! - **Epoch drift**: [`Storage::bump_project_epoch`] is called once per index
//!   run that changed data; maps record the epoch they were created at, so
//!   `current - created` measures how much the project moved on even when no
//!   direct anchor broke.
//! - **Anchor invalidation**: each `cited` anchor stores the chunk content
//!   hash observed at persist time. When the current hash for that
//!   `(buffer_id, path)` differs, the map becomes `stale` with a granular
//!   JSON `stale_reason` listing the broken paths.

use anyhow::Context as _;
use anyhow::Result;
use rusqlite::OptionalExtension as _;
use rusqlite::params;
use std::collections::BTreeMap;

use super::super::conn::Storage;
use super::super::tokens::now_ms;

/// One anchor whose stored hash no longer matches the current chunk hash.
#[derive(Debug, Clone)]
pub struct BrokenAnchor {
    /// Numeric rowid of the affected exploration.
    pub id: i64,
    /// Stable UUIDv7 id of the affected exploration.
    pub exploration_id: String,
    /// Path whose hash changed (for `stale_reason`).
    pub path: String,
}

impl Storage {
    /// Return the current monotone epoch for a project (0 if never bumped).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn current_project_epoch(&self, project: &str) -> Result<i64> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                "SELECT COALESCE((SELECT epoch FROM project_epochs WHERE project = ?1), 0)",
                params![project],
                |r| r.get(0),
            )
            .context("failed to read project epoch")
        })
    }

    /// Atomically increment and return the epoch for a project. Called by the
    /// post-index hook whenever an index run changed any data.
    ///
    /// # Errors
    ///
    /// Returns an error if the upsert fails.
    pub fn bump_project_epoch(&self, project: &str) -> Result<i64> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = now_ms();
        conn.execute(|c| {
            c.query_row(
                "INSERT INTO project_epochs (project, epoch, updated_at) VALUES (?1, 1, ?2) \
                 ON CONFLICT(project) DO UPDATE SET epoch = epoch + 1, updated_at = ?2 \
                 RETURNING epoch",
                params![project, now],
                |r| r.get(0),
            )
            .context("failed to bump project epoch")
        })
    }

    /// Mark every `fresh` map in a project stale when any of its `cited`
    /// anchors no longer matches the current chunk hash for its
    /// `(buffer_id, path)`. The broken paths are recorded granularly in
    /// `stale_reason` (JSON array). Returns the number of maps invalidated.
    ///
    /// Idempotent: already-stale/retired maps are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if any statement fails.
    pub fn mark_stale_if_anchors_changed(&self, project: &str) -> Result<usize> {
        let broken = self.broken_anchors(project)?;
        if broken.is_empty() {
            return Ok(0);
        }

        // rowid -> (exploration_id, set of broken paths)
        let mut by_map: BTreeMap<i64, (String, Vec<String>)> = BTreeMap::new();
        for b in &broken {
            by_map
                .entry(b.id)
                .or_insert_with(|| (b.exploration_id.clone(), Vec::new()))
                .1
                .push(b.path.clone());
        }

        let conn = self.connection().context("failed to acquire connection")?;
        let now = now_ms();
        conn.execute(|c| {
            let tx = c.unchecked_transaction().context("begin staleness tx")?;
            let mut invalidated = 0usize;
            for (rowid, (exploration_id, mut paths)) in by_map {
                paths.sort();
                paths.dedup();
                let reason = serde_json::to_string(&paths).context("serialize stale_reason")?;
                let n = tx
                    .execute(
                        "UPDATE explorations SET status = 'stale', stale_reason = ?1, \
                         updated_at = ?2 \
                         WHERE id = ?3 AND status = 'fresh'",
                        params![reason, now, rowid],
                    )
                    .context("mark exploration stale")?;
                invalidated += n;
                tracing::info!(
                    rowid,
                    exploration_id = %exploration_id,
                    project = %project,
                    broken = ?paths,
                    "exploration marked stale"
                );
            }
            tx.commit().context("commit staleness tx")?;
            Ok(invalidated)
        })
    }

    /// List every broken `cited` anchor for a project: anchors whose stored
    /// hash differs from the CURRENT chunk hash of `(buffer_id, file_path)`.
    /// Anchors whose file no longer has chunks count as broken (deleted).
    ///
    /// This is the read-time recheck primitive — cheap enough to run on every
    /// search hit because hits are rare.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn broken_anchors(&self, project: &str) -> Result<Vec<BrokenAnchor>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let sql = "SELECT e.id, e.exploration_id, f.path \
                       FROM explorations e \
                       JOIN exploration_files f ON f.exploration_rowid = e.id \
                       WHERE e.project = ?1 AND e.status != 'retired' AND f.role = 'cited' \
                       AND NOT EXISTS (\
                           SELECT 1 FROM chunks ch \
                           WHERE ch.buffer_id = f.buffer_id \
                             AND ch.file_path = f.path \
                             AND ch.hash = CAST(f.content_hash AS BLOB))";
            let mut stmt = c.prepare(sql).context("prepare broken_anchors")?;
            let rows = stmt
                .query_map(params![project], |r| {
                    Ok(BrokenAnchor {
                        id: r.get(0)?,
                        exploration_id: r.get(1)?,
                        path: r.get(2)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Recheck anchors for one specific map (read-time verification). Returns
    /// every currently-broken cited path; empty means all anchors hold.
    ///
    /// # Errors
    ///
    /// Returns an error if any query fails.
    pub fn recheck_anchors_for_rowid(&self, id: i64) -> Result<Vec<String>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let sql = "SELECT f.path FROM exploration_files f \
                       WHERE f.exploration_rowid = ?1 AND f.role = 'cited' \
                       AND NOT EXISTS (\
                           SELECT 1 FROM chunks ch \
                           WHERE ch.buffer_id = f.buffer_id \
                             AND ch.file_path = f.path \
                             AND ch.hash = CAST(f.content_hash AS BLOB))";
            let mut stmt = c.prepare(sql).context("prepare recheck anchors")?;
            let rows = stmt
                .query_map(params![id], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Resolve cited paths against the current index: for each path, the
    /// latest chunk content hash of `(buffer_id, path)`, or `None` when the
    /// file has no indexed chunks (unresolvable → cannot anchor).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn current_hashes_for_paths(
        &self,
        buffer_id: i64,
        paths: &[String],
    ) -> Result<Vec<(String, Option<String>)>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let sql = "SELECT DISTINCT hash FROM chunks \
                       WHERE buffer_id = ?1 AND file_path = ?2 LIMIT 1";
            let mut stmt = c.prepare(sql).context("prepare current_hashes_for_paths")?;
            let mut out = Vec::with_capacity(paths.len());
            for path in paths {
                let hash: Option<Option<String>> = stmt
                    .query_row(params![buffer_id, path], |r| {
                        let bytes: Vec<u8> = r.get(0)?;
                        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
                    })
                    .optional()
                    .with_context(|| format!("resolve hash for {path}"))?;
                out.push((path.clone(), hash.flatten()));
            }
            Ok(out)
        })
    }

    /// List `(buffer_id, path)` anchors of a map (provenance inspection).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_exploration_anchors(&self, rowid: i64) -> Result<Vec<(i64, String, String)>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let sql = "SELECT buffer_id, path, content_hash FROM exploration_files \
                       WHERE exploration_rowid = ?1 ORDER BY role DESC, path";
            let mut stmt = c.prepare(sql).context("prepare list anchors")?;
            let rows = stmt
                .query_map(params![rowid], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}
