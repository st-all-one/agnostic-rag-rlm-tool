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
    /// Whether this is the live revision (issue `agnostic-rlm-rs-e210`).
    pub is_active: bool,
    /// Rowid of the newer revision that superseded this one (`is_active = 0`
    /// rows only); `None` for the live row (issue `agnostic-rlm-rs-e210`).
    pub superseded_by: Option<i64>,
    /// Project epoch at write time (drift / time-travel, plan 021).
    pub epoch: i64,
    /// Agent username that stored the answer (audit/provenance).
    pub created_by: Option<String>,
    /// Revision counter; starts at 1, bumped on supersede (plan 021).
    pub version: i64,
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
    /// Authenticated session username that stored the answer (issue
    /// `agnostic-rlm-rs-786a`). `None` when the store is used outside an
    /// authenticated session (e.g. CLI hermetic paths).
    pub created_by: Option<String>,
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

/// Supersede-aware insert for a QA answer: retire any active row at
/// `(project, buffer_id, question_hash)` and insert a fresh active revision
/// (issue `agnostic-rlm-rs-e210`). Runs inside an open transaction so the
/// retire/insert pair commits atomically.
fn store_answer_inner(
    c: &rusqlite::Connection,
    input: &StoreAnswerInput,
    now: i64,
) -> Result<(String, i64)> {
    let tx = c.unchecked_transaction().context("begin store_answer tx")?;

    let existing: Option<(i64, i64)> = tx
        .query_row(
            "SELECT id, version FROM qa_cache \
             WHERE project = ?1 AND buffer_id IS ?2 AND question_hash = ?3 \
               AND is_active = 1 LIMIT 1",
            params![input.project, input.buffer_id, input.question_hash],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()
        .context("failed to probe existing qa_cache entry")?;

    let cache_id = uuid::Uuid::now_v7().to_string();
    let chunk_ids_json = serde_json::to_string(&input.source_chunk_ids)
        .context("failed to serialize source_chunk_ids")?;
    let hashes_json =
        serde_json::to_string(&input.source_hashes).context("failed to serialize source_hashes")?;
    let snapshot = input
        .tier_snapshot
        .clone()
        .unwrap_or_else(|| "{}".to_string());

    let new_id: i64 = match existing {
        Some((old_id, old_version)) => {
            // Retire the previous active row first so the partial unique index
            // (one active per subject) is never violated by the insert.
            tx.execute(
                "UPDATE qa_cache SET is_active = 0 WHERE id = ?1 AND is_active = 1",
                params![old_id],
            )
            .context("failed to retire superseded qa_cache row")?;
            let rowid: i64 = tx
                .query_row(
                    "INSERT INTO qa_cache \
                     (cache_id, buffer_id, project, question_text, question_hash, \
                      answer_text, source_chunk_ids, source_hashes, model, created_by, \
                      version, is_active, confidence, tier_snapshot, token_count, \
                      access_count, created_at, last_accessed_at, stale) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, 1.0, ?12, \
                             ?13, 0, ?14, ?14, 0) \
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
                        input.created_by,
                        old_version + 1,
                        snapshot,
                        input.token_count,
                        now,
                    ],
                    |r| r.get(0),
                )
                .context("failed to insert superseding qa_cache row")?;
            tx.execute(
                "UPDATE qa_cache SET superseded_by = ?1 WHERE id = ?2",
                params![rowid, old_id],
            )
            .context("failed to link superseded qa_cache row")?;
            rowid
        }
        None => tx
            .query_row(
                "INSERT INTO qa_cache \
                 (cache_id, buffer_id, project, question_text, question_hash, \
                  answer_text, source_chunk_ids, source_hashes, model, created_by, \
                  version, is_active, confidence, tier_snapshot, token_count, \
                  access_count, created_at, last_accessed_at, stale) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, 1, 1.0, ?11, ?12, 0, \
                         ?13, ?13, 0) \
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
                    input.created_by,
                    snapshot,
                    input.token_count,
                    now,
                ],
                |r| r.get(0),
            )
            .context("failed to insert qa_cache entry")?,
    };

    tx.commit().context("commit store_answer tx")?;
    Ok((cache_id, new_id))
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
        is_active: r.get::<_, i64>(20)? != 0,
        superseded_by: r.get(21)?,
        epoch: r.get(22)?,
        created_by: r.get(23)?,
        version: r.get(24)?,
    })
}

/// Projection shared by all `qa_cache` row queries (order fixed; see
/// [`row_mapper`]). The trailing `is_active` / `superseded_by` / `epoch` /
/// `created_by` / `version` are what the supersede chain walks for time-travel
/// (issue `agnostic-rlm-rs-e210` / plan 021).
const QA_COLS: &str = "id, cache_id, buffer_id, project, question_text, question_hash, \
     answer_text, source_chunk_ids, source_hashes, model, confidence, tier_snapshot, \
     token_count, access_count, created_at, last_accessed_at, stale, invalidated_at, \
     invalidated_by, invalidated_reason, is_active, superseded_by, epoch, created_by, version";

impl Storage {
    /// Store a digested answer, returning its stable `cache_id` and rowid.
    ///
    /// **Superseding (issue `agnostic-rlm-rs-e210`):** if an active row already
    /// exists for `(project, buffer_id, question_hash)` a *new* row is inserted
    /// (`version = old + 1`, `is_active = 1`) and the previous active row is
    /// retired (`is_active = 0`, `superseded_by = new_id`). Reads therefore see
    /// only the latest active revision, while the prior answer remains available
    /// through [`Storage::get_answer_history`]. No active row → a brand-new
    /// active row (`version = 1`) is inserted.
    ///
    /// The pre-existing staleness invalidation (`invalidate_stale_cache_for_buffer`
    /// / `mark_stale_by_hashes`) is preserved: a stale active row is still the
    /// "active" revision until superseded, so an exact-hit read treats it as a
    /// MISS and forces re-digest.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or any statement fails.
    pub fn store_answer(&self, input: &StoreAnswerInput) -> Result<StoredAnswer> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = now_ms();
        let start = std::time::Instant::now();
        let (cache_id, id) = conn
            .execute(|c| store_answer_inner(c, input, now))
            .context("store_answer tx")?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::debug!(
            phase = "store_answer",
            rowid = id,
            cache_id = %cache_id,
            buffer_id = input.buffer_id.map_or(-1, |b| b),
            project = %input.project,
            elapsed_ms = format!("{elapsed_ms:.2}"),
            "qa answer stored (superseding prior active revision)"
        );

        Ok(StoredAnswer {
            cache_id,
            id,
            created: true,
        })
    }

    /// Exact-hit lookup by `(project, question_hash)`. Returns `None` on miss
    /// or if the only match is stale (caller should treat stale as a MISS that
    /// forces re-digest). Only the latest **active** revision is considered.
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
                &format!(
                    "SELECT {QA_COLS} FROM qa_cache \
                         WHERE project = ?1 AND question_hash = ?2 AND is_active = 1 \
                           AND stale = 0 LIMIT 1"
                ),
                params![project, question_hash],
                row_mapper,
            )
            .optional()
            .context("failed to get cached answer")
        })
    }

    /// Time-travel: return the cached answer for `(project, question_hash,
    /// buffer_id)` that was **active** at `as_of_epoch`. The active revision at
    /// time T is the one with the greatest `epoch <= T` among every revision
    /// sharing that subject (newest → oldest). Staleness is ignored: an answer
    /// later marked stale was still the live answer at T, so it is the correct
    /// time-travel result. If no revision predates T, `None` is returned.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_cached_answer_as_of(
        &self,
        project: &str,
        question_hash: &str,
        buffer_id: Option<i64>,
        as_of_epoch: i64,
    ) -> Result<Option<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                &format!(
                    "SELECT {QA_COLS} FROM qa_cache \
                          WHERE project = ?1 AND buffer_id IS ?2 AND question_hash = ?3 \
                            AND epoch <= ?4 ORDER BY epoch DESC, id DESC LIMIT 1"
                ),
                params![project, buffer_id, question_hash, as_of_epoch],
                row_mapper,
            )
            .optional()
            .context("failed to get cached answer as_of")
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
                &format!(
                    "SELECT {QA_COLS} FROM qa_cache WHERE cache_id = ?1 AND project = ?2 LIMIT 1"
                ),
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
                &format!("SELECT {QA_COLS} FROM qa_cache WHERE cache_id = ?1 LIMIT 1"),
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
                &format!("SELECT {QA_COLS} FROM qa_cache WHERE id = ?1 LIMIT 1"),
                params![id],
                row_mapper,
            )
            .optional()
            .context("failed to get qa_cache by rowid")
        })
    }

    /// Walk the supersede chain starting from `id`, returning every revision in
    /// oldest→newest order (issue `agnostic-rlm-rs-e210`). The starting row need
    /// not be the oldest; only the forward chain reachable via `superseded_by`
    /// is returned. Retired (`is_active = 0`) revisions are included so callers
    /// can audit the full answer history.
    ///
    /// # Errors
    ///
    /// Returns an error if any query fails.
    pub fn get_answer_history(&self, id: i64) -> Result<Vec<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut chain = Vec::new();
            let mut current: Option<i64> = Some(id);
            while let Some(cid) = current {
                let Some(row) = c
                    .query_row(
                        &format!("SELECT {QA_COLS} FROM qa_cache WHERE id = ?1"),
                        params![cid],
                        row_mapper,
                    )
                    .optional()
                    .context("failed to read qa_cache history row")?
                else {
                    break;
                };
                let next: Option<i64> = c
                    .query_row(
                        "SELECT superseded_by FROM qa_cache WHERE id = ?1",
                        params![cid],
                        |r| r.get(0),
                    )
                    .context("failed to read qa_cache superseded_by")?;
                chain.push(row);
                current = next;
            }
            Ok(chain)
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
        // answer (issue `agnostic-rlm-rs-d172`). Best-effort: a failure to
        // enqueue must not roll back the staleness mark.
        if n > 0 {
            if let Err(e) = self.enqueue_pending_qa_for_stale(id) {
                tracing::warn!(error = %e, qa_id = id, "failed to enqueue re-digest job on stale");
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

    /// Mark the given QA cache rows as awaiting vector re-derivation.
    ///
    /// Sets `vector_status = 'pending_vector'` for every id in `cache_ids`. The
    /// canonical question text is preserved, so a reconcile worker
    /// (issue `agnostic-rlm-rs-36ae`) can re-embed.
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
            c.execute(&sql, rusqlite::params_from_iter(params.iter()))
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

    /// Return `(id, question_text)` pairs for the given QA cache rows, used by
    /// the reconcile worker (`agnostic-rlm-rs-36ae`) to re-embed the canonical
    /// question text from SQLite. Missing rows are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_qa_embed_inputs(&self, ids: &[i64]) -> Result<Vec<(i64, String)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let placeholders: Vec<String> = ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect();
            let sql = format!(
                "SELECT id, question_text FROM qa_cache WHERE id IN ({})",
                placeholders.join(", ")
            );
            let mut stmt = c.prepare(&sql).context("prepare qa embed inputs query")?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .context("query qa embed inputs")?;
            let mut out = Vec::with_capacity(ids.len());
            for row in rows {
                out.push(row.context("read qa embed input")?);
            }
            Ok(out)
        })
    }

    /// Return `(id, question_text)` pairs for **every** QA cache row, used by
    /// the server bootstrap rebuild (`agnostic-rlm-rs-620d`) to reconstruct the
    /// question vector space from SQLite when it diverges from the store.
    /// Missing rows are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn all_qa_embed_inputs(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare("SELECT id, question_text FROM qa_cache")
                .context("prepare all qa embed inputs query")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .context("query all qa embed inputs")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.context("read all qa embed input")?);
            }
            Ok(out)
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
            c.execute(&sql, rusqlite::params_from_iter(params.iter()))
                .context("clear_qa_cache_pending_vector")?;
            Ok(())
        })
    }
}
