//! Semantic query-answer cache persistence (plan 017).
//!
//! The server stores *digested* answers here (synthesized client-side). Lookup
//! by `(project, question_hash)` gives exact hits; similarity hits are resolved
//! by the caller against the dedicated `question_vectors` index
//! (`crate::qa_vectors`). `source_hashes` drive staleness: when a source chunk
//! changes, the lifecycle hook marks the row `stale` so the next query forces a
//! re-digest.
//!
//! All queries go through [`super::conn::Storage::connection`], which is safe in
//! both single (CLI) and pooled (server) modes.

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use rusqlite::params;

use super::conn::Storage;
use super::tokens::now_ms;

/// A stored query-answer cache row.
#[derive(Debug, Clone)]
pub struct QaCacheRow {
    /// Numeric rowid (also the `question_vectors` key).
    pub id: i64,
    /// Stable UUIDv7 answer id (anti-drift, propagated to sub-agents).
    pub cache_id: String,
    /// Scoping buffer id (project).
    pub buffer_id: Option<i64>,
    /// Project name (redundant for fast lookup).
    pub project: String,
    /// Original question text.
    pub question_text: String,
    /// Exact-hit hash of the question.
    pub question_hash: String,
    /// Digested answer text.
    pub answer_text: String,
    /// Provenance: chunk ids that produced the answer.
    pub source_chunk_ids: Vec<String>,
    /// Invalidation: content hashes of source chunks.
    pub source_hashes: Vec<String>,
    /// LLM model that synthesized (metadata).
    pub model: Option<String>,
    /// Confidence (decays to 0 when stale).
    pub confidence: f64,
    /// Thresholds snapshot (JSON, for reproducibility).
    pub tier_snapshot: Option<String>,
    /// Token cost of the answer.
    pub token_count: i64,
    /// Access count (for weighted LRU eviction).
    pub access_count: i64,
    /// Created epoch ms.
    pub created_at: i64,
    /// Last accessed epoch ms.
    pub last_accessed_at: i64,
    /// Whether the entry is stale.
    pub stale: bool,
    /// Epoch ms of manual invalidation (audit).
    pub invalidated_at: Option<i64>,
    /// Who invalidated (audit).
    pub invalidated_by: Option<String>,
    /// Why invalidated (audit).
    pub invalidated_reason: Option<String>,
}

/// Input for [`Storage::store_answer`].
#[derive(Debug, Clone)]
pub struct StoreAnswerInput {
    /// Scoping buffer id.
    pub buffer_id: Option<i64>,
    /// Project name.
    pub project: String,
    /// Original question text.
    pub question_text: String,
    /// Exact-hit hash of the question.
    pub question_hash: String,
    /// Digested answer text.
    pub answer_text: String,
    /// Provenance: chunk ids.
    pub source_chunk_ids: Vec<String>,
    /// Invalidation: chunk content hashes.
    pub source_hashes: Vec<String>,
    /// LLM model (metadata).
    pub model: Option<String>,
    /// Thresholds snapshot (JSON).
    pub tier_snapshot: Option<String>,
    /// Token cost.
    pub token_count: i64,
}

/// Result of storing an answer.
#[derive(Debug, Clone)]
pub struct StoredAnswer {
    /// Stable answer id.
    pub cache_id: String,
    /// Numeric rowid (question_vectors key).
    pub id: i64,
    /// Whether this was a brand-new entry (vs. an idempotent reuse).
    pub created: bool,
}

/// Compute the exact-hit hash for a question (normalized, lowercased).
#[must_use]
pub fn question_hash(question: &str) -> String {
    use sha2::{Digest, Sha256};
    let normalized: String = question
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    hex::encode(hasher.finalize())
}

/// Canonical content hash for a chunk (sha256 hex). Clients must use this exact
/// function when computing `source_hashes` so the server's staleness hook can
/// compare against stored chunk hashes.
///
/// Re-exported from [`arags_core::qa_cache::chunk_content_hash`] so client and
/// server share one implementation (plan 020: CLI has no storage dependency).
#[must_use]
pub fn chunk_content_hash(content: &str) -> String {
    arags_core::qa_cache::chunk_content_hash(content)
}

/// Parse a JSON array column into a `Vec<String>`.
fn parse_json_array(text: Option<String>) -> Vec<String> {
    match text {
        Some(s) if !s.is_empty() => serde_json::from_str::<Vec<String>>(&s).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn row_mapper(r: &rusqlite::Row<'_>) -> rusqlite::Result<QaCacheRow> {
    Ok(QaCacheRow {
        id: r.get(0)?,
        cache_id: r.get(1)?,
        buffer_id: r.get(2)?,
        project: r.get(3)?,
        question_text: r.get(4)?,
        question_hash: r.get(5)?,
        answer_text: r.get(6)?,
        source_chunk_ids: parse_json_array(r.get::<_, Option<String>>(7)?),
        source_hashes: parse_json_array(r.get::<_, Option<String>>(8)?),
        model: r.get(9)?,
        confidence: r.get(10)?,
        tier_snapshot: r.get(11)?,
        token_count: r.get(12)?,
        access_count: r.get(13)?,
        created_at: r.get(14)?,
        last_accessed_at: r.get(15)?,
        stale: r.get::<_, i64>(16)? != 0,
        invalidated_at: r.get(17)?,
        invalidated_by: r.get(18)?,
        invalidated_reason: r.get(19)?,
    })
}

impl Storage {
    /// Store a digested answer, returning its stable `cache_id` and rowid.
    ///
    /// **Idempotent / reserve-lock:** if a non-stale entry already exists for
    /// `(project, question_hash)`, its `cache_id`/`id` are returned without
    /// inserting a duplicate (concurrent identical MISSes reuse one entry).
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn store_answer(&self, input: &StoreAnswerInput) -> Result<StoredAnswer> {
        let conn = self.connection().context("failed to acquire connection")?;

        // Reserve-lock: reuse an existing non-stale entry for this question.
        if let Some(existing) = conn.execute(|c| {
            c.query_row(
                "SELECT id, cache_id FROM qa_cache \
                 WHERE project = ?1 AND question_hash = ?2 AND stale = 0 \
                 LIMIT 1",
                params![input.project, input.question_hash],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .context("failed to probe existing qa_cache entry")
        })? {
            return Ok(StoredAnswer {
                cache_id: existing.1,
                id: existing.0,
                created: false,
            });
        }

        let now = now_ms();
        let cache_id = uuid::Uuid::now_v7().to_string();
        let chunk_ids_json = serde_json::to_string(&input.source_chunk_ids)
            .context("failed to serialize source_chunk_ids")?;
        let hashes_json = serde_json::to_string(&input.source_hashes)
            .context("failed to serialize source_hashes")?;
        let snapshot = input
            .tier_snapshot
            .clone()
            .unwrap_or_else(|| "{}".to_string());

        let id: i64 = conn.execute(|c| {
            c.query_row(
                "INSERT INTO qa_cache \
                 (cache_id, buffer_id, project, question_text, question_hash, answer_text, \
                  source_chunk_ids, source_hashes, model, confidence, tier_snapshot, \
                  token_count, access_count, created_at, last_accessed_at, stale) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1.0, ?10, ?11, 0, ?12, ?12, 0) \
                 RETURNING id",
                params![
                    cache_id,
                    input.buffer_id,
                    input.project,
                    input.question_text,
                    input.question_hash,
                    input.answer_text,
                    chunk_ids_json,
                    hashes_json,
                    input.model,
                    snapshot,
                    input.token_count,
                    now,
                ],
                |r| r.get::<_, i64>(0),
            )
            .context("failed to insert qa_cache entry")
        })?;

        Ok(StoredAnswer {
            cache_id,
            id,
            created: true,
        })
    }

    /// Exact-hit lookup by `(project, question_hash)`. Returns `None` on miss
    /// or if the only match is stale (caller should treat stale as a MISS that
    /// forces re-digest).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_cached_answer(
        &self,
        project: &str,
        question_hash: &str,
    ) -> Result<Option<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                "SELECT id, cache_id, buffer_id, project, question_text, question_hash, \
                 answer_text, source_chunk_ids, source_hashes, model, confidence, \
                 tier_snapshot, token_count, access_count, created_at, last_accessed_at, \
                 stale, invalidated_at, invalidated_by, invalidated_reason \
                 FROM qa_cache WHERE project = ?1 AND question_hash = ?2 AND stale = 0 \
                 LIMIT 1",
                params![project, question_hash],
                row_mapper,
            )
            .optional()
            .context("failed to get cached answer")
        })
    }

    /// Look up a cached answer by its stable `cache_id` (scoped by project).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_qa_by_id(&self, cache_id: &str, project: &str) -> Result<Option<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                "SELECT id, cache_id, buffer_id, project, question_text, question_hash, \
                 answer_text, source_chunk_ids, source_hashes, model, confidence, \
                 tier_snapshot, token_count, access_count, created_at, last_accessed_at, \
                 stale, invalidated_at, invalidated_by, invalidated_reason \
                 FROM qa_cache WHERE cache_id = ?1 AND project = ?2 LIMIT 1",
                params![cache_id, project],
                row_mapper,
            )
            .optional()
            .context("failed to get qa_cache by id")
        })
    }

    /// Look up a cached answer by its stable `cache_id` globally (admin
    /// invalidation does not require the caller to know the project).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_qa_by_cache_id(&self, cache_id: &str) -> Result<Option<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                "SELECT id, cache_id, buffer_id, project, question_text, question_hash, \
                 answer_text, source_chunk_ids, source_hashes, model, confidence, \
                 tier_snapshot, token_count, access_count, created_at, last_accessed_at, \
                 stale, invalidated_at, invalidated_by, invalidated_reason \
                 FROM qa_cache WHERE cache_id = ?1 LIMIT 1",
                params![cache_id],
                row_mapper,
            )
            .optional()
            .context("failed to get qa_cache by cache_id")
        })
    }

    /// Look up a cached answer by numeric rowid (used by radius invalidation).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_qa_by_rowid(&self, id: i64) -> Result<Option<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                "SELECT id, cache_id, buffer_id, project, question_text, question_hash, \
                 answer_text, source_chunk_ids, source_hashes, model, confidence, \
                 tier_snapshot, token_count, access_count, created_at, last_accessed_at, \
                 stale, invalidated_at, invalidated_by, invalidated_reason \
                 FROM qa_cache WHERE id = ?1 LIMIT 1",
                params![id],
                row_mapper,
            )
            .optional()
            .context("failed to get qa_cache by rowid")
        })
    }

    /// Mark an entry stale (soft invalidation) and record the audit trail.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn mark_qa_stale(&self, id: i64, invalidated_by: &str, reason: &str) -> Result<bool> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = now_ms();
        conn.execute(|c| {
            let n = c.execute(
                "UPDATE qa_cache SET stale = 1, confidence = 0, \
                 invalidated_at = ?1, invalidated_by = ?2, invalidated_reason = ?3 \
                 WHERE id = ?4 AND stale = 0",
                params![now, invalidated_by, reason, id],
            )?;
            Ok(n)
        })
        .context("failed to mark qa_cache stale")?;
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
        let now = now_ms();
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
        let now = now_ms();
        let lambda = if lambda_ms <= 0 { 1 } else { lambda_ms };
        conn.execute(|c| {
            let count: i64 = c.query_row(
                "SELECT COUNT(*) FROM qa_cache WHERE project = ?1",
                params![project],
                |r| r.get(0),
            )?;
            if count <= i64::try_from(max_entries).unwrap_or(i64::MAX) {
                return Ok(0);
            }
            // Score ascending; delete the excess lowest-scoring rows.
            let excess = count - i64::try_from(max_entries).unwrap_or(i64::MAX);
            let mut stmt = c.prepare(
                "SELECT id FROM qa_cache WHERE project = ?1 \
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
