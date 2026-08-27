//! QA-cache eviction and staleness-on-reindex handlers.

use anyhow::{Context, Result};
use rusqlite::params;

use super::row::parse_json_array;
use crate::sqlite::conn::Storage;

impl Storage {
    /// Weighted-LRU eviction: drop the lowest-scoring entries for a project until
    /// `count <= max_entries`. Score = access_count / (1 + age/lambda).
    ///
    /// # Errors
    ///
    /// Returns an error if the query/delete fails.
    pub fn evict_qa(&self, project: &str, max_entries: usize, lambda_ms: i64) -> Result<usize> {
        if max_entries == 0 {
            // Keep at least one slot; 0 would purge everything.
            return Ok(0);
        }
        let conn = self.connection().context("failed to acquire connection")?;
        let now = crate::sqlite::tokens::now_ms();
        let lambda = if lambda_ms <= 0 { 1 } else { lambda_ms };
        conn.execute(|c| {
            let count: i64 = c.query_row(
                "SELECT COUNT(*) FROM qa_cache WHERE project = ?1 AND is_active = 1",
                params![project],
                |r| r.get(0),
            )?;
            if count <= i64::try_from(max_entries).unwrap_or(i64::MAX) {
                return Ok(0);
            }
            // Score ascending; delete the excess lowest-scoring rows.
            let excess = count - i64::try_from(max_entries).unwrap_or(i64::MAX);
            let mut stmt = c.prepare(
                "SELECT id FROM qa_cache WHERE project = ?1 AND is_active = 1 \
                 ORDER BY (access_count * 1.0) / (1.0 + ((?2 - last_accessed_at) * 1.0 / ?3)) ASC \
                 LIMIT ?4",
            )?;
            let ids: Vec<i64> = stmt
                .query_map(params![project, now, lambda, excess], |r| r.get(0))?
                .filter_map(std::result::Result::ok)
                .collect();
            let n = ids.len();
            for id in ids {
                c.execute("DELETE FROM qa_cache WHERE id = ?1", params![id])?;
            }
            Ok(n)
        })
        .context("failed to evict qa_cache entries")
    }

    /// Count cached entries for a project (any staleness).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn count_qa(&self, project: &str) -> Result<usize> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM qa_cache WHERE project = ?1",
                params![project],
                |r| r.get(0),
            )?;
            Ok(usize::try_from(n).unwrap_or(0))
        })
        .context("failed to count qa_cache entries")
    }

    /// List every `qa_cache.id` (used by full-project / full purge to also
    /// remove the corresponding question vectors).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn all_qa_ids(&self) -> Result<Vec<i64>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c.prepare("SELECT id FROM qa_cache")?;
            let ids = stmt
                .query_map([], |r| r.get::<_, i64>(0))?
                .filter_map(std::result::Result::ok)
                .collect();
            Ok(ids)
        })
        .context("failed to list qa_cache ids")
    }

    /// Run weighted-LRU eviction across all projects.
    ///
    /// # Errors
    ///
    /// Returns an error if any per-project eviction fails.
    pub fn evict_all_qa(&self, max_entries: usize, lambda_ms: i64) -> Result<usize> {
        let conn = self.connection().context("failed to acquire connection")?;
        let projects: Vec<String> = conn.execute(|c| {
            let mut stmt = c.prepare("SELECT DISTINCT project FROM qa_cache")?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .filter_map(std::result::Result::ok)
                .collect();
            Ok(rows)
        })?;
        let mut total = 0;
        for p in projects {
            total += self.evict_qa(&p, max_entries, lambda_ms)?;
        }
        Ok(total)
    }

    /// List `(id, source_hashes)` for a buffer (used by the staleness hook).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_qa_hashes_for_buffer(&self, buffer_id: i64) -> Result<Vec<(i64, Vec<String>)>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut stmt =
                c.prepare("SELECT id, source_hashes FROM qa_cache WHERE buffer_id = ?1")?;
            let rows = stmt
                .query_map(params![buffer_id], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        parse_json_array(r.get::<_, Option<String>>(1)?),
                    ))
                })?
                .filter_map(std::result::Result::ok)
                .collect();
            Ok(rows)
        })
        .context("failed to list qa hashes for buffer")
    }

    /// Mark every non-stale entry in a buffer stale whose `source_hashes`
    /// reference a chunk that no longer exists (post-reindex staleness hook).
    ///
    /// # Errors
    ///
    /// Returns an error if the queries fail.
    pub fn invalidate_stale_cache_for_buffer(&self, buffer_id: i64) -> Result<usize> {
        let current = self.chunk_hashes_for_buffer(buffer_id).unwrap_or_default();
        if current.is_empty() {
            return Ok(0);
        }
        let rows = self.list_qa_hashes_for_buffer(buffer_id)?;
        let mut n = 0;
        for (id, hashes) in rows {
            let missing = hashes.iter().any(|h| !current.contains(h));
            if missing && self.mark_qa_stale(id, "system", "chunk changed")? {
                n += 1;
            }
        }
        Ok(n)
    }
}
